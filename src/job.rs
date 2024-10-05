use std::io;
use std::path::Path;
use std::process::{Command, ExitStatus};

pub struct Job<'a> {
    inner: imp::Job<'a>,
}

impl<'a> Job<'a> {
    pub fn new(path: &'a Path, args: &'a [String], elevated: bool) -> Self {
        Self {
            inner: imp::Job::new(path, args, elevated),
        }
    }

    pub fn run(&mut self) -> io::Result<()> {
        self.inner.run()
    }

    pub fn command(&mut self) -> Option<&mut Command> {
        self.inner.command()
    }
}

fn check_exit_code(code: i64) -> io::Result<()> {
    if code != 0 {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!("exit code: {} (0x{:X})", code, code),
        ));
    }
    Ok(())
}

fn check_exit_status(status: ExitStatus) -> io::Result<()> {
    if let Some(code) = status.code() {
        check_exit_code(code as _)
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub struct Job<'a> {
        cmd: Command,
        _marker: std::marker::PhantomData<&'a ()>,
    }

    impl<'a> Job<'a> {
        pub fn new(path: &Path, args: &[String], _elevated: bool) -> Self {
            let mut cmd = Command::new(path);
            cmd.args(args);
            Self {
                cmd,
                _marker: std::marker::PhantomData,
            }
        }

        pub fn run(&mut self) -> io::Result<()> {
            let mut child = self.cmd.spawn()?;
            check_exit_status(child.wait()?)
        }

        pub fn command(&mut self) -> Option<&mut Command> {
            Some(&mut self.cmd)
        }
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use crate::INTERRUPT_EVENT;
    use std::ffi::c_void;
    use std::ffi::{OsStr, OsString};
    use std::io;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::os::windows::io::FromRawHandle;
    use std::os::windows::prelude::RawHandle;
    use std::os::windows::{
        io::{AsRawHandle, OwnedHandle},
        process::CommandExt,
    };
    use std::path::PathBuf;
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::Foundation::{
        HANDLE, HINSTANCE, HWND, INVALID_HANDLE_VALUE, WAIT_EVENT, WAIT_FAILED, WAIT_OBJECT_0,
    };
    use windows::Win32::System::JobObjects::*;
    use windows::Win32::System::Registry::HKEY;
    use windows::Win32::System::SystemServices::JOB_OBJECT_MSG_ACTIVE_PROCESS_ZERO;
    use windows::Win32::System::Threading::*;
    use windows::Win32::System::IO::{
        CreateIoCompletionPort, GetQueuedCompletionStatus, OVERLAPPED,
    };
    use windows::Win32::UI::Shell::*;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    fn interrupt_event() -> io::Result<HANDLE> {
        // Get the event handle associated with Ctrl+C.
        // TODO: decouple from the INTERRUPT_EVENT global var.
        Ok(INTERRUPT_EVENT
            .lock()
            .map_err(|e| {
                io::Error::new(
                    io::ErrorKind::Other,
                    format!("Failed to take interrupt lock: {}", e),
                )
            })?
            .event
            .0)
    }

    unsafe fn to_owned(handle: HANDLE) -> OwnedHandle {
        OwnedHandle::from_raw_handle(RawHandle::from(handle.0))
    }

    /// Look up the executable associated with a file.
    fn get_associated_command(path: &OsStr) -> Option<PathBuf> {
        let mut app_path: Vec<u16> = vec![0; 4096];
        let mut app_path_length: u32 = app_path.len() as u32;

        let wide_path: Vec<u16> = path.encode_wide().chain(Some(0)).collect();

        let result = unsafe {
            AssocQueryStringW(
                ASSOCF_NOTRUNCATE | ASSOCF_REMAPRUNDLL,
                ASSOCSTR_EXECUTABLE,
                PCWSTR(wide_path.as_ptr()),
                None,
                PWSTR(app_path.as_mut_ptr()),
                &mut app_path_length,
            )
        };

        if result.is_ok() {
            let launcher = OsString::from_wide(&app_path[..app_path_length as usize - 1]);
            if launcher.to_string_lossy().starts_with("%") {
                None
            } else {
                Some(PathBuf::from(launcher))
            }
        } else {
            None
        }
    }

    /// Retrieve the handle of the main thread for a process id.
    fn get_main_thread_handle(pid: u32) -> io::Result<OwnedHandle> {
        use windows::Win32::System::Diagnostics::ToolHelp::*;
        unsafe {
            // Take a snapshot of the system
            let snapshot = to_owned(CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0)?);
            let handle = HANDLE(snapshot.as_raw_handle());

            let mut thread_entry = THREADENTRY32 {
                dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
                ..Default::default()
            };

            // Get the first thread
            if Thread32First(handle, &mut thread_entry).is_ok() {
                loop {
                    if thread_entry.th32OwnerProcessID == pid {
                        // Found a thread belonging to our process
                        let thread_handle =
                            OpenThread(THREAD_ALL_ACCESS, false, thread_entry.th32ThreadID)?;
                        return Ok(to_owned(thread_handle));
                    }

                    // Move to the next thread
                    if Thread32Next(handle, &mut thread_entry).is_err() {
                        break;
                    }
                }
            }

            Err(io::Error::last_os_error())
        }
    }

    /// Create job and add process (expected to have been started with CREATE_SUSPENDED).
    /// Set JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE.
    fn add_process_to_job(pid: u32, proc: HANDLE) -> io::Result<OwnedHandle> {
        let main_thread = get_main_thread_handle(pid)?;

        let job = unsafe { to_owned(CreateJobObjectW(None, None)?) };
        unsafe {
            let mut job_info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            job_info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                HANDLE(job.as_raw_handle()),
                JobObjectExtendedLimitInformation,
                &mut job_info as *mut _ as *mut _,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )?;

            AssignProcessToJobObject(HANDLE(job.as_raw_handle()), proc)?;

            // Resume the process.
            ResumeThread(HANDLE(main_thread.as_raw_handle()));
        }
        Ok(job)
    }

    pub struct Job<'a> {
        cmd: Option<Command>,
        path: &'a Path,
        args: &'a [String],
    }

    impl<'a> Job<'a> {
        pub fn new(path: &'a Path, args: &'a [String], elevated: bool) -> Self {
            let cmd = if elevated {
                None
            } else {
                Some(Self::create_command(path, args))
            };

            Self { cmd, path, args }
        }

        pub fn run(&mut self) -> io::Result<()> {
            match self.cmd.as_mut() {
                Some(command) => Self::run_command(command),
                None => self.runas(),
            }
        }

        fn runas(&self) -> io::Result<()> {
            let verb: Vec<u16> = OsStr::new("runas").encode_wide().chain(Some(0)).collect();
            let file: Vec<u16> = self.path.as_os_str().encode_wide().chain(Some(0)).collect();

            let args = self.args.join(" ");
            let params: Vec<u16> = OsStr::new(&args).encode_wide().chain(Some(0)).collect();

            let mut sei = SHELLEXECUTEINFOW {
                cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
                fMask: SEE_MASK_NOCLOSEPROCESS,
                hwnd: HWND::default(),
                lpVerb: PCWSTR(verb.as_ptr()),
                lpFile: PCWSTR(file.as_ptr()),
                lpParameters: PCWSTR(params.as_ptr()),
                lpDirectory: PCWSTR::null(),
                nShow: SW_SHOWNORMAL.0,
                hInstApp: HINSTANCE::default(),
                lpIDList: std::ptr::null_mut(),
                lpClass: PCWSTR::null(),
                hkeyClass: HKEY::default(),
                dwHotKey: 0,
                Anonymous: SHELLEXECUTEINFOW_0::default(),
                hProcess: HANDLE::default(),
            };

            unsafe {
                ShellExecuteExW(&mut sei)?;

                // TODO: can this happen?
                // if sei.hProcess.is_invalid() {
                //     return Err(io::Error::last_os_error());
                // }
                assert!(!sei.hProcess.is_invalid());

                let process = to_owned(sei.hProcess);

                // This does not work:
                // let pid = GetProcessId(HANDLE(process.as_raw_handle()));
                // let job = add_process_to_job(pid, sei.hProcess)?;
                // Self::wait(&job)?;
                let handles = [HANDLE(process.as_raw_handle()), interrupt_event()?];
                let wait_result = WaitForMultipleObjects(&handles, false, INFINITE);

                if wait_result == WAIT_FAILED {
                    return Err(io::Error::last_os_error());
                } else if wait_result == WAIT_EVENT(WAIT_OBJECT_0.0 + 1) {
                    _ = TerminateProcess(sei.hProcess, 2);
                }

                let mut exit_code: u32 = 0;
                GetExitCodeProcess(sei.hProcess, &mut exit_code)?;

                check_exit_code(exit_code as _)
            }
        }

        fn run_command(command: &mut Command) -> io::Result<()> {
            let mut child = command.spawn()?;

            let job = add_process_to_job(child.id(), HANDLE(child.as_raw_handle()))?;
            Self::wait(&job)?;

            check_exit_status(child.wait()?)
        }

        /// Wait for all processes associated with the Job object to complete.
        fn wait(job: &OwnedHandle) -> io::Result<()> {
            let iocp = Self::create_completion_port(&job)?;

            let handles = [HANDLE(iocp.as_raw_handle()), interrupt_event()?];

            unsafe {
                let mut completion_code: u32 = 0;
                let mut completion_key: usize = 0;
                let mut overlapped: *mut OVERLAPPED = std::ptr::null_mut();

                loop {
                    let wait_res = WaitForMultipleObjects(&handles, false, INFINITE);
                    if wait_res == WAIT_OBJECT_0 {
                        GetQueuedCompletionStatus(
                            HANDLE(iocp.as_raw_handle()),
                            &mut completion_code,
                            &mut completion_key,
                            &mut overlapped,
                            0,
                        )?;
                        if completion_key == job.as_raw_handle() as usize
                            && completion_code == JOB_OBJECT_MSG_ACTIVE_PROCESS_ZERO
                        {
                            break;
                        }
                    } else if wait_res == WAIT_EVENT(WAIT_OBJECT_0.0 + 1) {
                        break;
                    } else {
                        eprintln!("{:?}", wait_res);
                        break;
                    }
                }
            }
            Ok(())
        }

        /// Return the command associated with the Job.
        pub fn command(&mut self) -> Option<&mut Command> {
            self.cmd.as_mut()
        }

        /// Create a std::process::Command to launch the process.
        /// If the path does not have EXE extension, look up the
        /// associated app; if not found, fail over to CMD.EXE /C
        fn create_command(path: &Path, args: &[String]) -> Command {
            let is_exe = path
                .extension()
                .map(|ext| ext.to_string_lossy().to_lowercase())
                .filter(|e| e == "exe")
                .is_some();

            let mut command = if is_exe {
                Command::new(path)
            } else {
                if let Some(launcher) = get_associated_command(path.as_os_str()) {
                    let mut command = Command::new(launcher);
                    command.arg(path).args(args);
                    command
                } else {
                    // Fail over to using CMD.EXE /C as launcher.
                    let mut command = Command::new("cmd");
                    command.arg("/C").arg(path).args(args);
                    command
                }
            };

            command.args(args).creation_flags(CREATE_SUSPENDED.0);

            command
        }

        /// Create a IO completion port and associate it with the Job object.
        fn create_completion_port(job: &OwnedHandle) -> io::Result<OwnedHandle> {
            unsafe {
                let iocp = to_owned(CreateIoCompletionPort(INVALID_HANDLE_VALUE, None, 0, 1)?);

                let port = JOBOBJECT_ASSOCIATE_COMPLETION_PORT {
                    CompletionKey: job.as_raw_handle(),
                    CompletionPort: HANDLE(iocp.as_raw_handle()),
                };

                SetInformationJobObject(
                    HANDLE(job.as_raw_handle()),
                    JobObjectAssociateCompletionPortInformation,
                    &port as *const _ as *const c_void,
                    std::mem::size_of::<JOBOBJECT_ASSOCIATE_COMPLETION_PORT>() as u32,
                )?;

                Ok(iocp)
            }
        }
    }
}
