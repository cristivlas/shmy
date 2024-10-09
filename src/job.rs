use crate::scope::Scope;
use std::io;
use std::path::Path;
use std::process::Command;

/// Execute commands as part of a Job. Experimental.
/// Just a simple std::process::Command wrapper for non-Windows targets.

pub struct Job<'a> {
    inner: imp::Job<'a>,
}

impl<'a> Job<'a> {
    pub fn new(scope: &'a Scope, path: &'a Path, args: &'a [String], elevated: bool) -> Self {
        Self {
            inner: imp::Job::new(scope, path, args, elevated),
        }
    }

    pub fn run(&mut self) -> io::Result<()> {
        self.inner.run()
    }

    pub fn command_mut(&mut self) -> Option<&mut Command> {
        self.inner.command_mut()
    }

    pub fn cmd_line(&mut self) -> Option<String> {
        if let Some(command) = self.inner.command_mut() {
            let cmd = std::iter::once(command.get_program())
                .chain(command.get_args())
                .map(|a| a.to_string_lossy().to_string())
                .collect::<Vec<_>>()
                .join(" ");

            return Some(cmd);
        }
        None
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
        pub fn new(_: &Scope, path: &Path, args: &[String], _elevated: bool) -> Self {
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

        pub fn command_mut(&mut self) -> Option<&mut Command> {
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
    use std::fs::File;
    use std::io::{self, Read};
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::os::windows::prelude::RawHandle;
    use std::os::windows::{
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
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

    const PE_SIGNATURE: &[u8] = b"PE\0\0";

    // Subsystem constants based on Windows PE header definitions
    const IMAGE_SUBSYSTEM_WINDOWS_GUI: u16 = 2;
    const IMAGE_SUBSYSTEM_WINDOWS_CUI: u16 = 3;

    #[repr(C)]
    struct PeFileHeader {
        machine: u16,
        number_of_sections: u16,
        timestamp: u32,
        pointer_to_symbol_table: u32,
        number_of_symbols: u32,
        size_of_optional_header: u16,
        characteristics: u16,
    }

    #[repr(C)]
    struct PeOptionalHeader {
        magic: u16,
        major_linker_version: u8,
        minor_linker_version: u8,
        size_of_code: u32,
        size_of_initialized_data: u32,
        size_of_uninitialized_data: u32,
        address_of_entry_point: u32,
        base_of_code: u32,
        base_of_data: u32,
        image_base: u32,
        section_alignment: u32,
        file_alignment: u32,
        major_operating_system_version: u16,
        minor_operating_system_version: u16,
        major_image_version: u16,
        minor_image_version: u16,
        major_subsystem_version: u16,
        minor_subsystem_version: u16,
        win32_version_value: u32,
        size_of_image: u32,
        size_of_headers: u32,
        checksum: u32,
        subsystem: u16,
        dll_characteristics: u16,
        size_of_stack_reserve: u64,
        size_of_stack_commit: u64,
        size_of_heap_reserve: u64,
        size_of_heap_commit: u64,
        loader_flags: u32,
        number_of_rva_and_sizes: u32,
    }

    #[derive(Debug)]
    enum Subsystem {
        Unknown,
        Console,
        GUI,
    }

    impl Default for Subsystem {
        fn default() -> Self {
            Self::Unknown
        }
    }

    /// Determine if executable is GUI or Console.
    fn get_exe_subsystem<P: AsRef<Path>>(path: P) -> io::Result<Subsystem> {
        let mut file = File::open(path)?;
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer)?;

        // Check for DOS header
        if buffer.len() < 64 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "File too small to be a valid PE",
            ));
        }

        // Check the DOS header and get the PE header offset
        let pe_offset = u32::from_le_bytes(buffer[60..64].try_into().unwrap_or_default()) as usize;

        // Check for PE signature
        if &buffer[pe_offset..pe_offset + 4] != PE_SIGNATURE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid PE signature",
            ));
        }

        // Optional Header starts after the PE signature (4 bytes) and the File Header (20 bytes)
        let optional_header_offset = pe_offset + 4 + size_of::<PeFileHeader>();

        // Check if there are enough bytes for the optional header
        if optional_header_offset + size_of::<PeOptionalHeader>() > buffer.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "File too small for Optional Header",
            ));
        }

        // Read the Optional Header from the buffer
        let optional_header_bytes =
            &buffer[optional_header_offset..optional_header_offset + size_of::<PeOptionalHeader>()];
        let optional_header: &PeOptionalHeader =
            unsafe { &*(optional_header_bytes.as_ptr() as *const _) };

        // Validate: check for 32 or 64 PE header magic.
        match optional_header.magic {
            0x010B | 0x20B => {}
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "Invalid magic number in PE header: 0x{:X}",
                        optional_header.magic
                    ),
                ));
            }
        }

        let subsystem = optional_header.subsystem;
        // println!("subsystem: {}", subsystem);
        match subsystem {
            IMAGE_SUBSYSTEM_WINDOWS_GUI => Ok(Subsystem::GUI),
            IMAGE_SUBSYSTEM_WINDOWS_CUI => Ok(Subsystem::Console),
            _ => Ok(Subsystem::Unknown),
        }
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
                dwSize: size_of::<THREADENTRY32>() as u32,
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

    /// $__limit_job_memory: max job memory in MB
    /// $__limit_proc_memory: max process memory in MB
    /// $__limit_proc_count: limit the number of processes associated with the job.
    /// TODO: complete with more variables
    /// TODO: write ulimit-like utility to manage and list these limits.
    fn apply_job_limits(scope: &Scope, job_info: &mut JOBOBJECT_EXTENDED_LIMIT_INFORMATION) {
        if let Some(limit) = scope
            .lookup("__limit_job_memory")
            .and_then(|v| v.value().as_str().parse::<usize>().ok())
        {
            job_info.JobMemoryLimit = limit * 1024 * 1024;
            job_info.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_JOB_MEMORY;
        }

        if let Some(limit) = scope
            .lookup("__limit_proc_memory")
            .and_then(|v| v.value().as_str().parse::<usize>().ok())
        {
            job_info.ProcessMemoryLimit = limit * 1024 * 1024;
            job_info.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_PROCESS_MEMORY;
        }

        if let Some(limit) = scope
            .lookup("__limit_proc_count")
            .and_then(|v| v.value().as_str().parse::<u32>().ok())
        {
            job_info.BasicLimitInformation.ActiveProcessLimit = limit;
            job_info.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
        }
    }

    /// Create job and add process (expected to have been started with CREATE_SUSPENDED).
    /// Set JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE.
    fn add_process_to_job(scope: &Scope, pid: u32, proc: HANDLE) -> io::Result<OwnedHandle> {
        let main_thread = get_main_thread_handle(pid)?;

        let job = unsafe { to_owned(CreateJobObjectW(None, None)?) };
        unsafe {
            let mut job_info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            job_info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

            apply_job_limits(scope, &mut job_info);

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
        scope: &'a Scope,
    }

    impl<'a> Job<'a> {
        pub fn new(scope: &'a Scope, path: &'a Path, args: &'a [String], elevated: bool) -> Self {
            let mut job = Self {
                cmd: None,
                path,
                args,
                exe: Cow::Borrowed(path),
                scope,
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
        /// Return the exit code.
        fn run_command(&mut self) -> io::Result<i64> {
            // This is a convoluted hack to determine how to handle Ctrl+C.
            // If the launched command is a Console App, do not send it CTRL_C_EVENT
            // nor terminate, assuming it implements its own handler (e.g. Python interpreter).
            // Terminate GUI apps on Ctrl+C -- in the future this may change to send WM_CLOSE.
            let kill_on_ctrl_c = matches!(
                get_exe_subsystem(&self.exe).unwrap_or_default(),
                Subsystem::GUI
            );

            let mut command = self.command().expect("No command");
            let mut child = command.spawn()?;

            let handle = HANDLE(child.as_raw_handle());

            // Set up cleanup machinery to terminate the child process in case
            // anything goes wrong with add_process_to_job. Maybe overkill?
            // struct Cleanup {
            //     process: Option<HANDLE>,
            // }
            // impl Drop for Cleanup {
            //     fn drop(&mut self) {
            //         if let Some(process) = self.process {
            //             unsafe {
            //                 _ = TerminateProcess(process, 42);
            //             }
            //         }
            //     }
            // }
            // let mut cleanup = Cleanup {
            //     process: Some(handle),
            // };

            let job = add_process_to_job(self.scope, child.id(), handle)?;
            // cleanup.process.take(); // cancel cleaning up the process, as it is now associated with the job

            // eprintln!("Waiting for job completion...");
            Self::wait(&job, kill_on_ctrl_c)?;

            drop(job);

            // eprintln!("Waiting for child process...");
            let status = child.wait()?;
            // eprintln!("Done.");

            if let Some(code) = status.code() {
                Ok(code as _)
            } else {
                Ok(0)
            }
        }

        /// Wait for all processes associated with the Job object to complete.
        fn wait(job: &OwnedHandle, kill_on_ctrl_c: bool) -> io::Result<()> {
            let iocp = Self::create_completion_port(&job)?;

            let handles = [HANDLE(iocp.as_raw_handle()), interrupt_event()?];
            let mut completion_code: u32 = 0;
            let mut completion_key: usize = 0;
            let mut overlapped: *mut OVERLAPPED = std::ptr::null_mut();

            unsafe {
                loop {
                    // Check that there are processes left in the job.
                    // let mut info = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
                    // QueryInformationJobObject(
                    //     HANDLE(job.as_raw_handle()),
                    //     JobObjectBasicAccountingInformation,
                    //     &mut info as *mut _ as *mut _,
                    //     std::mem::size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                    //     None,
                    // )?;
                    // if info.TotalProcesses == 0 {
                    //     break;
                    // }
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
                        if kill_on_ctrl_c {
                            // Terminating is not strictly needed, dropping the job should be enough
                            // but this way the user gets to see an error (exit code 2).
                            _ = TerminateJobObject(HANDLE(job.as_raw_handle()), 2);
                            break;
                        }
                    } else {
                        eprintln!("{:?}", wait_res);
                        break;
                    }
                }
            }
            Ok(())
        }

        /// Return the command associated with the Job.
        pub fn command_mut(&mut self) -> Option<&mut Command> {
            self.cmd.as_mut()
        }

        pub fn command(&mut self) -> Option<Command> {
            self.cmd.take()
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
                    size_of::<JOBOBJECT_ASSOCIATE_COMPLETION_PORT>() as u32,
                )?;

                Ok(iocp)
            }
        }
    }
}
