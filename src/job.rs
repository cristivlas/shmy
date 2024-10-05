use std::io;
use std::path::Path;
use std::process::{Command, ExitStatus};

pub struct Job {
    inner: imp::Job,
}

impl Job {
    pub fn new(path: &Path, args: &[String], elevated: bool) -> Self {
        Self {
            inner: imp::Job::new(path, args, elevated),
        }
    }

    pub fn run(&mut self) -> io::Result<ExitStatus> {
        self.inner.run()
    }

    pub fn command(&mut self) -> Option<&mut Command> {
        self.inner.command()
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub struct Job {
        cmd: Command,
    }

    impl Job {
        pub fn new(path: &Path, args: &[String], _elevated: bool) -> Self {
            let mut cmd = Command::new(path);
            cmd.args(args);
            Self { cmd }
        }

        pub fn run(&mut self) -> io::Result<ExitStatus> {
            let mut child = self.cmd.spawn()?;
            child.wait()
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
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::os::windows::io::FromRawHandle;
    use std::os::windows::prelude::RawHandle;
    use std::os::windows::{
        io::{AsRawHandle, OwnedHandle},
        process::CommandExt,
    };
    use std::path::PathBuf;
    use std::process::Child;
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE, WAIT_EVENT, WAIT_OBJECT_0};
    use windows::Win32::System::JobObjects::*;
    use windows::Win32::System::SystemServices::JOB_OBJECT_MSG_ACTIVE_PROCESS_ZERO;
    use windows::Win32::System::Threading::*;
    use windows::Win32::System::IO::{
        CreateIoCompletionPort, GetQueuedCompletionStatus, OVERLAPPED,
    };
    use windows::Win32::UI::Shell::*;

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
    fn main_thread_handle(pid: u32) -> io::Result<OwnedHandle> {
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
    fn add_process_to_job(process: &Child) -> io::Result<OwnedHandle> {
        let main_thread = main_thread_handle(process.id())?;

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

            AssignProcessToJobObject(HANDLE(job.as_raw_handle()), HANDLE(process.as_raw_handle()))?;

            // Resume the process.
            ResumeThread(HANDLE(main_thread.as_raw_handle()));
        }
        Ok(job)
    }

    pub struct Job {
        cmd: Option<Command>,
    }

    impl Job {
        pub fn new(path: &Path, args: &[String], elevated: bool) -> Self {
            let mut job = Self { cmd: None };

            if elevated {
                todo!() // TODO
            } else {
                job.cmd = Some(Self::create_command(path, args));
            }

            job
        }

        pub fn run(&mut self) -> io::Result<ExitStatus> {
            match self.cmd.as_mut() {
                Some(command) => Self::run_command(command),
                None => {
                    todo!(); // TODO: elevated mode
                }
            }
        }

        fn run_command(command: &mut Command) -> io::Result<ExitStatus> {
            let mut child = command.spawn()?;

            let job = add_process_to_job(&child)?;
            let iocp = Self::completion_port(&job)?;

            // Get the event handle associated with Ctrl+C.
            // TODO: decouple from the INTERRUPT_EVENT global var.
            let interrupt = INTERRUPT_EVENT
                .lock()
                .map_err(|e| {
                    io::Error::new(
                        io::ErrorKind::Other,
                        format!("Failed to take interrupt lock: {}", e),
                    )
                })?
                .event
                .0;

            let handles = [HANDLE(iocp.as_raw_handle()), interrupt];

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
                        let process = HANDLE(child.as_raw_handle());
                        _ = TerminateProcess(process, 2);
                        break;
                    } else {
                        break; // TODO: error handling?
                    }
                }

                child.wait()
            }
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
        fn completion_port(job: &OwnedHandle) -> io::Result<OwnedHandle> {
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
