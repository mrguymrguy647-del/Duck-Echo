//! A Windows "job object": a box of programs that Windows stops all
//! together.
//!
//! The Ear is put in a job made with `KILL_ON_JOB_CLOSE`. Every program the
//! Ear starts is put in the same job by Windows. When Duck Echo stops the
//! job, or when Duck Echo itself ends and Windows closes its handle to the
//! job, every program in it is stopped.
//!
//! This is the only `unsafe` code in Duck Echo: Rust cannot check what a
//! Windows function does with the values it is given, so each call says
//! why it is safe.
#![allow(unsafe_code)]

use std::ffi::c_void;
use std::io;
use std::os::windows::io::AsRawHandle;
use std::process::Child;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

/// A job object that stops all its programs when it is closed.
pub struct Job(HANDLE);

// SAFETY: a job's handle may be used from any thread (Windows says so for
// all kernel handles), and `Job` never hands it out.
unsafe impl Send for Job {}

impl Job {
    /// A new, empty job that kills its programs when closed.
    pub fn new() -> io::Result<Job> {
        // SAFETY: both pointers may be null (no security settings, no
        // name); Windows returns a new handle or null.
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        let job = Job(handle); // from here on, dropping it closes the handle
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: `job.0` is the job made above; the pointer and the size
        // describe `limits`, which lives until the call returns.
        let done = unsafe {
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                &limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION as *const c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if done == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(job)
    }

    /// Put `child` (and so every program it starts later) in the job.
    pub fn assign(&self, child: &Child) -> io::Result<()> {
        // SAFETY: both handles are open: the job's is ours, and `child`'s
        // stays open while `child` is borrowed.
        let done = unsafe { AssignProcessToJobObject(self.0, child.as_raw_handle() as HANDLE) };
        if done == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    /// Stop every program in the job.
    pub fn stop_all(&self) {
        // SAFETY: the job's handle is open until `drop`. A failure (the
        // programs are already gone) needs nothing done.
        unsafe { TerminateJobObject(self.0, 1) };
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        // SAFETY: the handle is open and is closed only here, once.
        unsafe { CloseHandle(self.0) };
    }
}
