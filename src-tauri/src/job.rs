use std::io;
use std::process::{Child, Command};

#[derive(Debug)]
pub struct ProcessJob {
    #[cfg(windows)]
    handle: usize,
}

impl ProcessJob {
    /// Assign the suspended root before any of its code can create descendants.
    pub fn spawn(command: &mut Command) -> io::Result<(Child, Self)> {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            use windows_sys::Win32::System::Threading::{CREATE_NO_WINDOW, CREATE_SUSPENDED};
            command.creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED);
        }
        let mut child = command.spawn()?;
        let result = Self::attach(&child).and_then(|job| {
            #[cfg(windows)]
            resume_owned_root(child.id())?;
            Ok(job)
        });
        match result {
            Ok(job) => Ok((child, job)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(error)
            }
        }
    }

    pub fn is_empty(&self) -> io::Result<bool> {
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::JobObjects::{
                JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JobObjectBasicAccountingInformation,
                QueryInformationJobObject,
            };
            let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { std::mem::zeroed() };
            let ok = unsafe {
                QueryInformationJobObject(
                    self.handle as _,
                    JobObjectBasicAccountingInformation,
                    (&raw mut info).cast(),
                    u32::try_from(std::mem::size_of_val(&info)).expect("job info fits u32"),
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(info.ActiveProcesses == 0)
        }
        #[cfg(not(windows))]
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "process tree observation requires Windows",
        ))
    }

    pub fn terminate(&self) -> io::Result<()> {
        #[cfg(windows)]
        {
            if unsafe {
                windows_sys::Win32::System::JobObjects::TerminateJobObject(self.handle as _, 1)
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }
        #[cfg(not(windows))]
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "process tree termination requires Windows",
        ))
    }

    pub fn attach(child: &Child) -> io::Result<Self> {
        #[cfg(windows)]
        {
            use windows_sys::Win32::Foundation::CloseHandle;
            use windows_sys::Win32::System::JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
                SetInformationJobObject,
            };
            use windows_sys::Win32::System::Threading::{
                OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA,
                PROCESS_TERMINATE,
            };

            let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if job.is_null() {
                return Err(io::Error::last_os_error());
            }
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let configured = unsafe {
                SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    (&raw mut limits).cast(),
                    u32::try_from(std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())
                        .expect("Windows JOBOBJECT_EXTENDED_LIMIT_INFORMATION size fits u32"),
                )
            };
            if configured == 0 {
                unsafe { CloseHandle(job) };
                return Err(io::Error::last_os_error());
            }

            let process = unsafe {
                OpenProcess(
                    PROCESS_SET_QUOTA | PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION,
                    0,
                    child.id(),
                )
            };
            if process.is_null() {
                unsafe { CloseHandle(job) };
                return Err(io::Error::last_os_error());
            }
            let assigned = unsafe { AssignProcessToJobObject(job, process) };
            unsafe { CloseHandle(process) };
            if assigned == 0 {
                unsafe { CloseHandle(job) };
                return Err(io::Error::last_os_error());
            }
            Ok(Self {
                handle: job as usize,
            })
        }
        #[cfg(not(windows))]
        {
            let _ = child;
            Ok(Self {})
        }
    }
}

#[cfg(windows)]
fn resume_owned_root(process_id: u32) -> io::Result<()> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
    };
    use windows_sys::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let mut entry: THREADENTRY32 = unsafe { std::mem::zeroed() };
    entry.dwSize = u32::try_from(std::mem::size_of_val(&entry)).expect("thread entry fits u32");
    let mut found = unsafe { Thread32First(snapshot, &raw mut entry) } != 0;
    let mut result = Err(io::Error::new(
        io::ErrorKind::NotFound,
        "suspended root thread missing",
    ));
    while found {
        if entry.th32OwnerProcessID == process_id {
            let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
            if thread.is_null() {
                result = Err(io::Error::last_os_error());
            } else {
                result = if unsafe { ResumeThread(thread) } == u32::MAX {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(())
                };
                unsafe { CloseHandle(thread) };
            }
            break;
        }
        found = unsafe { Thread32Next(snapshot, &raw mut entry) } != 0;
    }
    unsafe { CloseHandle(snapshot) };
    result
}

#[cfg(windows)]
impl Drop for ProcessJob {
    fn drop(&mut self) {
        if self.handle != 0 {
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(self.handle as _);
            }
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    #[test]
    fn root_exit_does_not_release_a_live_descendant() {
        let mut command = Command::new("powershell.exe");
        command.args(["-NoProfile", "-Command", "Start-Process powershell.exe -WindowStyle Hidden -ArgumentList '-NoProfile -Command Start-Sleep -Seconds 30'"]);
        let (mut child, job) = ProcessJob::spawn(&mut command).unwrap();
        assert!(child.wait().unwrap().success());
        assert!(
            !job.is_empty().unwrap(),
            "root exit cannot prove tree completion"
        );
        job.terminate().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !job.is_empty().unwrap() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(job.is_empty().unwrap());
    }
}
