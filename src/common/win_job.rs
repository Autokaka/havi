use std::os::windows::io::RawHandle;
use std::sync::OnceLock;
use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, SetInformationJobObject, TerminateJobObject,
    JobObjectExtendedLimitInformation, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

struct JobHandle(HANDLE);
unsafe impl Send for JobHandle {}
unsafe impl Sync for JobHandle {}

fn job() -> Option<HANDLE> {
    static JOB: OnceLock<Option<JobHandle>> = OnceLock::new();
    JOB.get_or_init(|| unsafe {
        let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if h.is_null() { return None; }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ok = SetInformationJobObject(
            h,
            JobObjectExtendedLimitInformation,
            (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        if ok == 0 { return None; }
        Some(JobHandle(h))
    }).as_ref().map(|j| j.0)
}

pub fn assign(child_handle: RawHandle) -> bool {
    let Some(h) = job() else { return false; };
    unsafe { AssignProcessToJobObject(h, child_handle as HANDLE) != 0 }
}

pub fn terminate_all() {
    if let Some(h) = job() {
        unsafe { TerminateJobObject(h, 0); }
    }
}
