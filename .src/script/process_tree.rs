//! A script runtime's process and every process it starts, ended as one.
//!
//! A process the script starts inherits the runtime's standard error, so a
//! runtime killed alone leaves that pipe open for as long as the process
//! runs, and the drain, the step's last words and its drop wait with it
//! (ADR-0066, amendment 2026-11-20). So the runtime is started as the root
//! of a tree of its own, and the tree is ended as one:
//!
//! - on Windows in a job object of its own, placed in it while it is still
//!   suspended, so nothing it starts runs outside the job, which allows no
//!   breaking away; ended by terminating the job, which also kills what is
//!   in it when its last handle closes, so a node that ends without
//!   dropping its step ends the tree too;
//! - on Unix as the leader of a process group of its own, made before the
//!   runtime's program runs; ended by killing the group. A process that
//!   leaves the group on purpose, for a session of its own, leaves the tree.
//!
//! The one file in this crate that may hold unsafe code (ADR-0050,
//! amendment 2026-09-25): a job object, a suspended thread resumed and a
//! process group's signal are a C interface, reached no other way. Every
//! block says where its pointers come from, how long they live and who
//! frees what they hand back.
#![allow(unsafe_code)]

use std::io;
use std::process::{Child, Command};

/// A runtime's process, the root, and every process it starts: ended as
/// one, once, by [`ProcessTree::end`] or when it drops.
pub(super) struct ProcessTree {
    root: Child,
    /// The job the tree runs in; closed, it kills what is left in it.
    #[cfg(windows)]
    job: std::os::windows::io::OwnedHandle,
    ended: bool,
}

impl ProcessTree {
    /// `command` started as the root of a tree of its own. Its creation
    /// flags on Windows and its process group on Unix are this call's.
    ///
    /// # Errors
    /// The process cannot be started, as [`Command::spawn`] says it (a
    /// program that is not there is [`io::ErrorKind::NotFound`]), or on
    /// Windows its job cannot be made or the process placed in it and let
    /// run; a process started is ended before the error returns.
    pub(super) fn started(command: &mut Command) -> io::Result<Self> {
        #[cfg(windows)]
        let (root, job) = windows::started(command)?;
        #[cfg(unix)]
        let root = unix::started(command)?;
        Ok(Self {
            root,
            #[cfg(windows)]
            job,
            ended: false,
        })
    }

    /// The root's process, for its standard streams.
    pub(super) fn root(&mut self) -> &mut Child {
        &mut self.root
    }

    /// Every process in the tree killed and the root reaped, once. The root
    /// is reaped last, so on Unix the group it leads is still its own when
    /// the group is signalled.
    pub(super) fn end(&mut self) {
        if std::mem::replace(&mut self.ended, true) {
            return;
        }
        #[cfg(windows)]
        let killed = windows::terminated(&self.job);
        #[cfg(unix)]
        let killed = unix::killed(&self.root);
        if !killed {
            let _ = self.root.kill();
        }
        let _ = self.root.wait();
    }
}

impl Drop for ProcessTree {
    fn drop(&mut self) {
        self.end();
    }
}

/// Whether `process` still runs: what a test asks of a process the script
/// started once its tree has ended.
#[cfg(test)]
pub(super) fn running(process: u32) -> bool {
    #[cfg(windows)]
    let running = windows::running(process);
    #[cfg(unix)]
    let running = unix::running(process);
    running
}

#[cfg(windows)]
mod windows {
    use std::io;
    use std::os::windows::io::{AsRawHandle, HandleOrNull, OwnedHandle};
    use std::os::windows::process::CommandExt;
    use std::process::{Child, Command};
    use std::ptr;

    use windows_sys::Win32::Foundation::{ERROR_NO_MORE_ITEMS, ERROR_SUCCESS};
    use windows_sys::Win32::System::Diagnostics::ProcessSnapshotting::{
        HPSS, HPSSWALK, PSS_CAPTURE_THREADS, PSS_THREAD_ENTRY, PSS_WALK_THREADS,
        PssCaptureSnapshot, PssFreeSnapshot, PssWalkMarkerCreate, PssWalkMarkerFree,
        PssWalkSnapshot,
    };
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject,
    };
    use windows_sys::Win32::System::Threading::{
        CREATE_SUSPENDED, GetCurrentProcess, OpenThread, ResumeThread, THREAD_SUSPEND_RESUME,
    };

    /// `command` started suspended, placed in a job of its own, then let
    /// run: nothing it starts can run before it is in the job.
    pub(super) fn started(command: &mut Command) -> io::Result<(Child, OwnedHandle)> {
        let job = job()?;
        let mut root = command.creation_flags(CREATE_SUSPENDED).spawn()?;
        match placed(&job, &root).and_then(|()| resumed(&root)) {
            Ok(()) => Ok((root, job)),
            Err(failed) => {
                let _ = root.kill();
                let _ = root.wait();
                Err(failed)
            }
        }
    }

    /// A job that kills every process in it when its last handle closes,
    /// and lets none break away (no breakaway limit is set). Its handle is
    /// not inherited, so this process holds the only one.
    fn job() -> io::Result<OwnedHandle> {
        // SAFETY: both pointers are null: no security attributes, no name.
        // The handle handed back, null where the call failed, is owned by
        // the `OwnedHandle` made of it, which closes it.
        let made =
            unsafe { HandleOrNull::from_raw_handle(CreateJobObjectW(ptr::null(), ptr::null())) };
        let job = OwnedHandle::try_from(made).map_err(|_| io::Error::last_os_error())?;
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let size = u32::try_from(size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())
            .map_err(io::Error::other)?;
        // SAFETY: the job's handle is live for the call, owned by `job`;
        // `limits` is a local of the size given, read during the call only.
        let set = unsafe {
            SetInformationJobObject(
                job.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                size,
            )
        };
        if set == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(job)
    }

    fn placed(job: &OwnedHandle, root: &Child) -> io::Result<()> {
        // SAFETY: both handles are live for the call: the job's, owned by
        // `job`, and the process's, owned by `root`.
        let placed = unsafe { AssignProcessToJobObject(job.as_raw_handle(), root.as_raw_handle()) };
        if placed == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Every thread of `root` resumed. A process started suspended has one,
    /// its first, whose handle `Child` does not hand over, so it is read
    /// from a snapshot of that process's threads alone.
    fn resumed(root: &Child) -> io::Result<()> {
        let snapshot = Snapshot::of(root)?;
        let walk = Walk::new()?;
        let mut suspended = false;
        while let Some(thread) = walk.next(&snapshot)? {
            suspended |= resume(thread)?;
        }
        if suspended {
            Ok(())
        } else {
            Err(io::Error::other(
                "its first thread was not found to let it run",
            ))
        }
    }

    /// One process's threads as they were, freed when it drops.
    struct Snapshot(HPSS);

    impl Snapshot {
        fn of(root: &Child) -> io::Result<Self> {
            let mut snapshot: HPSS = ptr::null_mut();
            // SAFETY: the process's handle is live for the call, owned by
            // `root`; `snapshot` is a local Windows writes the snapshot's
            // handle into, freed when the `Snapshot` made of it drops.
            let captured = unsafe {
                PssCaptureSnapshot(
                    root.as_raw_handle(),
                    PSS_CAPTURE_THREADS,
                    0,
                    &raw mut snapshot,
                )
            };
            succeeded(captured)?;
            Ok(Self(snapshot))
        }
    }

    impl Drop for Snapshot {
        fn drop(&mut self) {
            // SAFETY: the snapshot was captured into this process, which
            // `GetCurrentProcess` names, and it is freed once, here.
            unsafe { PssFreeSnapshot(GetCurrentProcess(), self.0) };
        }
    }

    /// A walk over a snapshot's threads, freed when it drops.
    struct Walk(HPSSWALK);

    impl Walk {
        fn new() -> io::Result<Self> {
            let mut walk: HPSSWALK = ptr::null_mut();
            // SAFETY: a null allocator asks for Windows' own; `walk` is a
            // local Windows writes the marker's handle into, freed when the
            // `Walk` made of it drops.
            succeeded(unsafe { PssWalkMarkerCreate(ptr::null(), &raw mut walk) })?;
            Ok(Self(walk))
        }

        /// The next thread's identifier, or none once all were walked.
        fn next(&self, snapshot: &Snapshot) -> io::Result<Option<u32>> {
            let mut entry = PSS_THREAD_ENTRY::default();
            let size = u32::try_from(size_of::<PSS_THREAD_ENTRY>()).map_err(io::Error::other)?;
            // SAFETY: both handles are live for the call, owned by
            // `snapshot` and `self`; `entry` is a local of the size given,
            // written by the call alone.
            let walked = unsafe {
                PssWalkSnapshot(
                    snapshot.0,
                    PSS_WALK_THREADS,
                    self.0,
                    (&raw mut entry).cast(),
                    size,
                )
            };
            if walked == ERROR_NO_MORE_ITEMS {
                return Ok(None);
            }
            succeeded(walked)?;
            Ok(Some(entry.ThreadId))
        }
    }

    impl Drop for Walk {
        fn drop(&mut self) {
            // SAFETY: the marker is this `Walk`'s, freed once, here.
            unsafe { PssWalkMarkerFree(self.0) };
        }
    }

    /// A process snapshot call's answer, an error code, as a result.
    fn succeeded(code: u32) -> io::Result<()> {
        if code == ERROR_SUCCESS {
            return Ok(());
        }
        Err(io::Error::from_raw_os_error(code.cast_signed()))
    }

    /// `thread` resumed; whether it was suspended.
    fn resume(thread: u32) -> io::Result<bool> {
        // SAFETY: no pointers. The handle handed back, null where the call
        // failed, is owned by the `OwnedHandle` made of it, which closes it.
        let opened =
            unsafe { HandleOrNull::from_raw_handle(OpenThread(THREAD_SUSPEND_RESUME, 0, thread)) };
        let thread = OwnedHandle::try_from(opened).map_err(|_| io::Error::last_os_error())?;
        // SAFETY: the thread's handle is live for the call, owned by `thread`.
        let before = unsafe { ResumeThread(thread.as_raw_handle()) };
        if before == u32::MAX {
            return Err(io::Error::last_os_error());
        }
        Ok(before > 0)
    }

    /// Every process in `job` killed; whether Windows took the call.
    pub(super) fn terminated(job: &OwnedHandle) -> bool {
        // SAFETY: the job's handle is live for the call, owned by `job`.
        unsafe { TerminateJobObject(job.as_raw_handle(), 1) != 0 }
    }

    #[cfg(test)]
    pub(super) fn running(process: u32) -> bool {
        use windows_sys::Win32::Foundation::WAIT_TIMEOUT;
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
        };
        // SAFETY: no pointers. The handle handed back, null where no such
        // process is, is owned by the `OwnedHandle` made of it.
        let opened =
            unsafe { HandleOrNull::from_raw_handle(OpenProcess(PROCESS_SYNCHRONIZE, 0, process)) };
        let Ok(process) = OwnedHandle::try_from(opened) else {
            return false;
        };
        // SAFETY: the process's handle is live for the call, owned by
        // `process`; a wait of 0 asks only whether it has ended.
        unsafe { WaitForSingleObject(process.as_raw_handle(), 0) == WAIT_TIMEOUT }
    }
}

#[cfg(unix)]
mod unix {
    use std::io;
    use std::os::unix::process::CommandExt;
    use std::process::{Child, Command};

    /// `command` started as the leader of a process group of its own, made
    /// in the child before the runtime's program runs.
    pub(super) fn started(command: &mut Command) -> io::Result<Child> {
        command.process_group(0).spawn()
    }

    /// Every process in the group `root` leads killed; whether the system
    /// took the call.
    pub(super) fn killed(root: &Child) -> bool {
        let Ok(group) = libc::pid_t::try_from(root.id()) else {
            return false;
        };
        // SAFETY: two integers, no pointer. The group is the root's own,
        // and its number cannot be another's: the root is reaped only after
        // this call.
        unsafe { libc::kill(-group, libc::SIGKILL) == 0 }
    }

    #[cfg(test)]
    pub(super) fn running(process: u32) -> bool {
        let Ok(process) = libc::pid_t::try_from(process) else {
            return false;
        };
        // SAFETY: two integers, no pointer; signal 0 sends nothing and asks
        // only whether the process is there.
        unsafe { libc::kill(process, 0) == 0 }
    }
}
