//! Keep std::Command's tested quoting/environment/pipe handling. Start suspended,
//! attach a Job, then resume the sole initial thread using stable Win32 APIs.
use ::windows::Win32::{
    Foundation::HANDLE,
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
        },
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject, TerminateJobObject,
        },
        Threading::{
            CREATE_NO_WINDOW, CREATE_SUSPENDED, OpenThread, ResumeThread, THREAD_SUSPEND_RESUME,
        },
    },
};
use anyhow::{Context, Result, bail};
use std::{
    os::windows::{
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
        process::CommandExt,
    },
    process::{Child, Command},
};

pub struct Job(OwnedHandle);
impl Job {
    fn new() -> Result<Self> {
        // SAFETY: no security descriptor/name; the returned handle is uniquely owned.
        let handle = unsafe { CreateJobObjectW(None, None)? };
        let job = Self(unsafe { OwnedHandle::from_raw_handle(handle.0) });
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: correctly sized live structure, valid owned job handle.
        unsafe {
            SetInformationJobObject(
                job.handle(),
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of_val(&limits) as u32,
            )?;
        }
        Ok(job)
    }
    fn handle(&self) -> HANDLE {
        HANDLE(self.0.as_raw_handle())
    }
    pub fn terminate(&self) {
        // SAFETY: the Job is owned by this instance, and contains only its child tree.
        let _ = unsafe { TerminateJobObject(self.handle(), 1) };
    }
}

fn resume(child: &Child) -> Result<()> {
    // Command does not expose the initial thread handle on stable Rust. The child
    // has not executed any code, so there must be exactly one thread in the snapshot.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0)? };
    let snapshot = unsafe { OwnedHandle::from_raw_handle(snapshot.0) };
    let handle = HANDLE(snapshot.as_raw_handle());
    let mut entry = THREADENTRY32 {
        dwSize: size_of::<THREADENTRY32>() as u32,
        ..Default::default()
    };
    let mut threads = Vec::new();
    // SAFETY: initialized size and live snapshot handle; iterate only its entries.
    unsafe {
        Thread32First(handle, &mut entry)?;
        loop {
            if entry.th32OwnerProcessID == child.id() {
                threads.push(entry.th32ThreadID);
            }
            entry.dwSize = size_of::<THREADENTRY32>() as u32;
            if Thread32Next(handle, &mut entry).is_err() {
                break;
            }
        }
    }
    if threads.len() != 1 {
        bail!("cannot identify the suspended client's initial thread");
    }
    // SAFETY: owned, still-live suspended process; only request resume access.
    let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, false, threads[0])? };
    let thread = unsafe { OwnedHandle::from_raw_handle(thread.0) };
    if unsafe { ResumeThread(HANDLE(thread.as_raw_handle())) } == u32::MAX {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

pub fn spawn(command: &mut Command) -> Result<(Child, Job)> {
    let job = Job::new()?;
    command.creation_flags(CREATE_SUSPENDED.0 | CREATE_NO_WINDOW.0);
    let mut child = {
        let _lock = super::SPAWN_LOCK
            .lock()
            .map_err(|_| anyhow::anyhow!("process spawn lock poisoned"))?;
        command.spawn()?
    };
    let result = (|| {
        // SAFETY: both owned handles remain live until after the association.
        unsafe {
            AssignProcessToJobObject(job.handle(), HANDLE(child.as_raw_handle()))?;
        }
        resume(&child)
    })();
    if let Err(error) = result {
        job.terminate();
        let _ = child.kill();
        let _ = child.wait();
        return Err(error).context("cannot attach/resume Windows client process tree");
    }
    Ok((child, job))
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use super::*;
    #[test]
    fn native_child_receives_exact_special_arguments() {
        let root = tempfile::tempdir().unwrap();
        let helper = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("ccsw-test-helper.exe");
        let copied = root.path().join("用户 & %literal% !^() helper.exe");
        std::fs::copy(helper, &copied).unwrap();
        let record = root.path().join("argv.json");
        let expected = [
            "--record",
            "",
            "a b",
            "用户 🦀",
            "a\"b",
            "C:\\trailing slash\\",
            "%PATH%",
            "!variable!",
            "& echo bad",
            "^()",
            "\\\\server\\share\\",
        ];
        let mut command = Command::new(copied);
        command.args(expected).env("CCSW_HELPER_RECORD", &record);
        let mut child = crate::managed_process::ManagedChild::spawn(&mut command).unwrap();
        assert!(child.wait().unwrap().success());
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(record).unwrap()).unwrap();
        assert_eq!(value["args"], serde_json::json!(expected));
    }
}
