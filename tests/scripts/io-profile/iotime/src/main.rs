//! Run each test in its own process inside a Windows job object and record
//! the job's aggregated CPU and I/O counters (which include every child
//! process the test spawned).
//!
//! Input: a TSV file, one task per line:
//!   exe<TAB>test_name<TAB>cwd
//!
//! Output: CSV beside the input, `<input>.csv`.

use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::os::windows::io::AsRawHandle;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOBOBJECT_BASIC_AND_IO_ACCOUNTING_INFORMATION,
    JOBOBJECT_BASIC_LIMIT_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JobObjectBasicAndIoAccountingInformation,
    JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
    TerminateJobObject,
};

struct Task {
    exe: String,
    name: String,
    cwd: String,
}

#[derive(Default)]
struct Sample {
    wall_ms: f64,
    exit_ms: f64,
    tail_ms: f64,
    cpu_total_ms: f64,
    cpu_period_ms: f64,
    read_ops: u64,
    write_ops: u64,
    other_ops: u64,
    read_bytes: u64,
    write_bytes: u64,
    other_bytes: u64,
    processes: u32,
    active_at_query: u32,
    exit: i32,
    assign_ok: bool,
    spawn_failed: bool,
}

unsafe fn make_job() -> HANDLE {
    let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
    if job.is_null() {
        panic!("CreateJobObjectW failed");
    }
    let mut ext: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
    ext.BasicLimitInformation = JOBOBJECT_BASIC_LIMIT_INFORMATION {
        LimitFlags: JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        ..std::mem::zeroed()
    };
    let ok = SetInformationJobObject(
        job,
        JobObjectExtendedLimitInformation,
        &ext as *const _ as *const core::ffi::c_void,
        std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
    );
    if ok == 0 {
        panic!("SetInformationJobObject failed");
    }
    job
}

unsafe fn query_job(job: HANDLE) -> JOBOBJECT_BASIC_AND_IO_ACCOUNTING_INFORMATION {
    let mut info: JOBOBJECT_BASIC_AND_IO_ACCOUNTING_INFORMATION = std::mem::zeroed();
    let mut ret = 0u32;
    QueryInformationJobObject(
        job,
        JobObjectBasicAndIoAccountingInformation,
        &mut info as *mut _ as *mut core::ffi::c_void,
        std::mem::size_of::<JOBOBJECT_BASIC_AND_IO_ACCOUNTING_INFORMATION>() as u32,
        &mut ret,
    );
    info
}

/// Read `ActiveProcesses` out of a job.
unsafe fn active_processes(job: HANDLE) -> u32 {
    // JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, laid out explicitly so a
    // mismatch with the binding cannot silently shift the field.
    #[repr(C)]
    struct BasicAccounting {
        total_user_time: i64,
        total_kernel_time: i64,
        this_period_total_user_time: i64,
        this_period_total_kernel_time: i64,
        total_page_fault_count: u32,
        total_processes: u32,
        active_processes: u32,
        total_terminated_processes: u32,
    }
    let mut info: BasicAccounting = std::mem::zeroed();
    let mut ret = 0u32;
    QueryInformationJobObject(
        job,
        windows_sys::Win32::System::JobObjects::JobObjectBasicAccountingInformation,
        &mut info as *mut _ as *mut core::ffi::c_void,
        std::mem::size_of::<BasicAccounting>() as u32,
        &mut ret,
    );
    info.active_processes
}

fn run_task(task: &Task) -> Sample {
    let job = unsafe { make_job() };
    let started = Instant::now();
    let child = Command::new(&task.exe)
        .arg("--exact")
        .arg(&task.name)
        .arg("--nocapture")
        .current_dir(&task.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            unsafe { CloseHandle(job) };
            eprintln!("spawn {}: {e}", task.exe);
            return Sample {
                spawn_failed: true,
                wall_ms: started.elapsed().as_secs_f64() * 1000.0,
                ..Default::default()
            };
        }
    };
    let assign_ok = unsafe {
        let raw = child.as_raw_handle() as HANDLE;
        AssignProcessToJobObject(job, raw) != 0
    };
    let status = child.wait();
    let exit_ms = started.elapsed().as_secs_f64() * 1000.0;
    let exit = status.map(|s| s.code().unwrap_or(-1)).unwrap_or(-1);

    // Give any grandchild the test left behind a moment to finish; the job's
    // counters cover the whole tree, so a live child's last writes are not
    // lost silently.
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut active;
    loop {
        active = unsafe { active_processes(job) };
        if active == 0 || Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }

    let info = unsafe { query_job(job) };
    let wall_ms = started.elapsed().as_secs_f64() * 1000.0;
    let sample = Sample {
        wall_ms,
        exit_ms,
        tail_ms: wall_ms - exit_ms,
        cpu_total_ms: (info.BasicInfo.TotalUserTime + info.BasicInfo.TotalKernelTime) as f64 / 10_000.0,
        cpu_period_ms: (info.BasicInfo.ThisPeriodTotalUserTime
            + info.BasicInfo.ThisPeriodTotalKernelTime) as f64
            / 10_000.0,
        read_ops: info.IoInfo.ReadOperationCount,
        write_ops: info.IoInfo.WriteOperationCount,
        other_ops: info.IoInfo.OtherOperationCount,
        read_bytes: info.IoInfo.ReadTransferCount,
        write_bytes: info.IoInfo.WriteTransferCount,
        other_bytes: info.IoInfo.OtherTransferCount,
        processes: info.BasicInfo.TotalProcesses,
        active_at_query: active,
        exit,
        assign_ok,
        spawn_failed: false,
    };
    unsafe {
        // Anything still alive is killed; the counters have been read.
        TerminateJobObject(job, 1);
        CloseHandle(job);
    }
    sample
}

fn main() {
    let mut args = std::env::args().skip(1);
    let list_path = args.next().expect("usage: iotime <tasks.tsv> [concurrency]");
    let concurrency: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(1);

    let file = File::open(&list_path).expect("open task list");
    let tasks: Vec<Task> = BufReader::new(file)
        .lines()
        .map(|l| l.expect("read line"))
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let mut parts = l.split('\t');
            Task {
                exe: parts.next().expect("exe").to_string(),
                name: parts.next().expect("name").to_string(),
                cwd: parts.next().expect("cwd").to_string(),
            }
        })
        .collect();

    let total = tasks.len();
    let queue = Arc::new(Mutex::new(tasks.into_iter().enumerate().collect::<Vec<_>>()));
    let done = Arc::new(Mutex::new(0usize));
    let out = Arc::new(Mutex::new({
        let mut f = File::create(format!("{list_path}.csv")).expect("create csv");
        writeln!(
            f,
            "exe,name,wall_ms,exit_ms,tail_ms,cpu_total_ms,cpu_period_ms,read_ops,write_ops,other_ops,read_bytes,write_bytes,other_bytes,processes,active_at_query,exit,assign_ok,spawn_failed"
        )
        .unwrap();
        f
    }));

    let mut handles = Vec::new();
    for _ in 0..concurrency.max(1) {
        let queue = queue.clone();
        let done = done.clone();
        let out = out.clone();
        handles.push(std::thread::spawn(move || loop {
            let item = {
                let mut q = queue.lock().unwrap();
                q.pop()
            };
            let Some((_, task)) = item else { break };
            let s = run_task(&task);
            {
                let mut f = out.lock().unwrap();
                writeln!(
                    f,
                    "{},{},{:.3},{:.3},{:.3},{:.3},{:.3},{},{},{},{},{},{},{},{},{},{},{}",
                    task.exe,
                    task.name,
                    s.wall_ms,
                    s.exit_ms,
                    s.tail_ms,
                    s.cpu_total_ms,
                    s.cpu_period_ms,
                    s.read_ops,
                    s.write_ops,
                    s.other_ops,
                    s.read_bytes,
                    s.write_bytes,
                    s.other_bytes,
                    s.processes,
                    s.active_at_query,
                    s.exit,
                    if s.assign_ok { 1 } else { 0 },
                    if s.spawn_failed { 1 } else { 0 }
                )
                .unwrap();
                let mut d = done.lock().unwrap();
                *d += 1;
                if *d % 50 == 0 {
                    eprintln!("{}/{}", *d, total);
                }
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    eprintln!("done {total} tests");
}
