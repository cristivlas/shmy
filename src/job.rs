/// Execute commands as part of a Job.
/// Just a simple std::process::Command wrapper for non-Windows targets.
use std::io;
use std::path::Path;
use std::process::Command;

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

#[cfg(not(windows))]
mod imp {
    use super::*;

    fn check_exit_status(status: std::process::ExitStatus) -> io::Result<()> {
        if let Some(code) = status.code() {
            check_exit_code(code as _)
        } else {
            Ok(())
        }
    }

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
    use crate::INTERRUPT_EVENT; // See interrupt_event function below.
    use std::borrow::Cow;
    use std::ffi::{c_void, OsStr, OsString};
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

    /// Get the event handle associated with Ctrl+C.
    /// TODO: decouple from the INTERRUPT_EVENT global var.
    fn interrupt_event() -> io::Result<HANDLE> {
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
    fn get_associated_command(path: &OsStr) -> Option<OsString> {
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
                Some(launcher)
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

            // Everything went okay so far. Resume the process.
            ResumeThread(HANDLE(main_thread.as_raw_handle()));
        }
        Ok(job)
    }

    const EXIT_CODE_EXEMPT: [&str; 2] = [
        "\\windows\\explorer.exe",
        "\\windows\\system32\\control.exe",
    ];

    pub struct Job<'a> {
        cmd: Option<Command>,
        path: &'a Path,
        args: &'a [String],
        exe: Cow<'a, Path>, // The actual executable that runs the command
    }

    impl<'a> Job<'a> {
        pub fn new(path: &'a Path, args: &'a [String], elevated: bool) -> Self {
            let mut job = Self {
                cmd: None,
                path,
                args,
                exe: Cow::Borrowed(path),
            };

            // Elevated (sudo) commands use ShellExecuteExW.
            if !elevated {
                job.create_command(path, args);
            }

            job
        }

        pub fn run(&mut self) -> io::Result<()> {
            let exit_code = if self.cmd.is_some() {
                self.run_command()
            } else {
                self.runas() // Run elevated (sudo)
            }?;

            // This is a hack for preventing errors for commands that are known to return
            // non-zero exit codes, such as the Control Panel (control.exe), that returns TRUE.
            // TODO: Come up with a better solution / workaround?
            for path in EXIT_CODE_EXEMPT {
                // Lowercase and skip the drive letter.
                if &self.exe.to_string_lossy().to_lowercase()[2..] == path {
                    return Ok(());
                }
            }
            check_exit_code(exit_code)
        }

        /// Run elevated. Used by the "sudo" command.
        fn runas(&self) -> io::Result<i64> {
            let verb: Vec<u16> = OsStr::new("runas").encode_wide().chain(Some(0)).collect();
            let file: Vec<u16> = self.path.as_os_str().encode_wide().chain(Some(0)).collect();

            let args = self.args.join(" ");
            let params: Vec<u16> = OsStr::new(&args).encode_wide().chain(Some(0)).collect();

            let mut sei = SHELLEXECUTEINFOW {
                cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
                fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
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

                // https://learn.microsoft.com/en-us/windows/win32/api/shellapi/ns-shellapi-shellexecuteinfow
                // Note: ShellExecuteEx does not always return an hProcess, even if a process is launched as the result of the call.
                if sei.hProcess.is_invalid() {
                    return Err(io::Error::new(
                        io::ErrorKind::Other,
                        "Invalid process handle",
                    ));
                }

                // Close the process automatically.
                let process = to_owned(sei.hProcess);

                let handles = [HANDLE(process.as_raw_handle()), interrupt_event()?];
                let wait_result = WaitForMultipleObjects(&handles, false, INFINITE);

                if wait_result == WAIT_FAILED {
                    return Err(io::Error::last_os_error());
                } else if wait_result == WAIT_EVENT(WAIT_OBJECT_0.0 + 1) {
                    _ = TerminateProcess(sei.hProcess, 2);
                }

                let mut exit_code: u32 = 0;
                GetExitCodeProcess(sei.hProcess, &mut exit_code)?;

                Ok(exit_code as _)
            }
        }

        /// Spawn command process and associate it with a job object.
        /// The process is created suspended and add_proccess_to_job resumes it on success.
        fn run_command(&mut self) -> io::Result<i64> {
            let command = self.command().expect("No command");
            let mut child = command.spawn()?;

            // Set up cleanup machinery to terminate the child process in case
            // anything goes wrong with add_process_to_job. Maybe overkill?
            struct Cleanup {
                process: Option<HANDLE>,
            }
            impl Drop for Cleanup {
                fn drop(&mut self) {
                    if let Some(process) = self.process {
                        unsafe {
                            _ = TerminateProcess(process, 42);
                        }
                    }
                }
            }
            let handle = HANDLE(child.as_raw_handle());
            let mut cleanup = Cleanup {
                process: Some(handle),
            };

            let job = add_process_to_job(child.id(), handle)?;
            cleanup.process.take(); // cancel the cleanup, the process is now associated with the job object

            Self::wait(&job)?;
            let status = child.wait()?;
            if let Some(code) = status.code() {
                Ok(code as _)
            } else {
                Ok(0)
            }
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
                    // Wait on the completion port and on the event that is set by Ctrl+C (see handles above).
                    let wait_res = WaitForMultipleObjects(&handles, false, INFINITE);

                    if wait_res == WAIT_OBJECT_0 {
                        // Woken up by the completion port? Check that all processes associated with the job are done.
                        // https://devblogs.microsoft.com/oldnewthing/20130405-00/?p=4743
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
                        // Ctrl+C event set. Do not TerminateProcess, just let the job go; that should finish the processes
                        // associated with the job object. Calling TerminateProcess may have undesired effects. For example:
                        // when running Python interactively from this shell, Ctrl+C should not terminate Python.
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
        fn create_command(&mut self, path: &Path, args: &[String]) {
            let is_exe = path
                .extension()
                .map(|ext| ext.to_string_lossy().to_lowercase())
                .filter(|e| e == "exe")
                .is_some();

            let mut command = if is_exe {
                Command::new(path)
            } else {
                if let Some(launcher) = get_associated_command(path.as_os_str()) {
                    self.exe = Cow::Owned(PathBuf::from(&launcher));

                    let mut command = Command::new(launcher);
                    command.arg(path).args(args);
                    command
                } else {
                    // Fail over to using CMD.EXE /C as launcher.
                    self.exe = Cow::Owned(PathBuf::from("cmd.exe"));

                    let mut command = Command::new("cmd");
                    command.arg("/C").arg(path).args(args);
                    command
                }
            };

            // Create the process suspended, so that it can be added to a Job object.
            command.args(args).creation_flags(CREATE_SUSPENDED.0);

            self.cmd = Some(command);
        }

        /// Create IO completion port and associate it with the Job object.
        /// https://devblogs.microsoft.com/oldnewthing/20130405-00/?p=4743
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
