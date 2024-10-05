use std::io;
use std::path::Path;
use std::process::{Command, ExitStatus};

pub struct Job {
    inner: imp::Job,
}

pub struct Signals {}

impl Job {
    pub fn new(path: &Path, args: &[String], elevated: bool) -> Self {
        Self {
            inner: imp::Job::new(path, args, elevated),
        }
    }

    pub fn run(&mut self, signals: Signals) -> io::Result<ExitStatus> {
        self.inner.run(signals)
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

        pub fn run(&mut self, _: Signals) -> io::Result<ExitStatus> {
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
    use std::ffi::{OsStr, OsString};
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::os::windows::io::FromRawHandle;
    use std::os::windows::prelude::RawHandle;
    use std::os::windows::{
        io::{AsRawHandle, OwnedHandle},
        process::CommandExt,
    };
    use std::path::PathBuf;
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::System::Threading::CREATE_SUSPENDED;
    use windows::Win32::System::Threading::*;
    use windows::Win32::UI::Shell::*;
    use windows::Win32::{
        Foundation::{HANDLE, WAIT_EVENT, WAIT_OBJECT_0},
        System::Threading::WaitForMultipleObjects,
    };

    unsafe fn to_owned(handle: HANDLE) -> OwnedHandle {
        OwnedHandle::from_raw_handle(RawHandle::from(handle.0))
    }

    /// Get the executable associated with a file.
    fn associated_command(path: &OsStr) -> Option<PathBuf> {
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

    /// Given a process id retrieve the handle of its main thread.
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
    /// The end goal is to have Ctrl+C kill all child processes the given proc. may have created.
    fn add_process_to_job(pid: u32) -> io::Result<OwnedHandle> {
        use windows::Win32::System::JobObjects::*;

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

            let proc = to_owned(OpenProcess(PROCESS_ALL_ACCESS, false, pid)?);
            AssignProcessToJobObject(HANDLE(job.as_raw_handle()), HANDLE(proc.as_raw_handle()))?;

            // Retrieve the main thread from the process and resume it.
            let thread = main_thread_handle(pid)?;
            ResumeThread(HANDLE(thread.as_raw_handle()));
        }
        Ok(job)
    }

    pub trait Interrupt {
        fn interrupt_event(&self) -> io::Result<HANDLE>;
    }

    impl Interrupt for Signals {
        fn interrupt_event(&self) -> io::Result<HANDLE> {
            use crate::INTERRUPT_EVENT;

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
    }

    pub struct Job {
        cmd: Option<Command>,
        job: Option<OwnedHandle>,
    }

    impl Job {
        pub fn new(path: &Path, args: &[String], elevated: bool) -> Self {
            let mut job = Self {
                cmd: None,
                job: None,
            };

            if elevated {
                todo!() // TODO
            } else {
                job.cmd = Some(Self::create_command(path, args));
            }

            job
        }

        pub fn run(&mut self, signals: Signals) -> io::Result<ExitStatus> {
            match self.cmd.as_mut() {
                Some(command) => {
                    let mut child = command.spawn()?;
                    self.job = Some(add_process_to_job(child.id())?);

                    let process = HANDLE(child.as_raw_handle());
                    let handles = [process, signals.interrupt_event()?];

                    unsafe {
                        let wait_result = WaitForMultipleObjects(&handles, false, INFINITE);
                        if wait_result == WAIT_EVENT(WAIT_OBJECT_0.0 + 1) {
                            _ = TerminateProcess(process, 2);
                        }
                    }
                    child.wait()
                }
                None => {
                    todo!(); // TODO: elevated mode
                }
            }
        }

        pub fn command(&mut self) -> Option<&mut Command> {
            self.cmd.as_mut()
        }

        fn create_command(path: &Path, args: &[String]) -> Command {
            let is_exe = path
                .extension()
                .map(|ext| ext.to_string_lossy().to_lowercase())
                .filter(|e| e == "exe")
                .is_some();

            let mut command = if is_exe {
                Command::new(path)
            } else {
                if let Some(launcher) = associated_command(path.as_os_str()) {
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
    }
}
