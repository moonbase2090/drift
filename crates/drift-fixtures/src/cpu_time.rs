use std::time::Duration;

use rustix::time::{clock_gettime, ClockId};

pub fn thread_cpu_time() -> Duration {
    let now = clock_gettime(ClockId::ThreadCPUTime);
    Duration::new(
        u64::try_from(now.tv_sec).unwrap_or(0),
        u32::try_from(now.tv_nsec).unwrap_or(0),
    )
}

pub fn cpu_cost(mut work: impl FnMut()) -> Duration {
    let started = thread_cpu_time();
    work();
    thread_cpu_time().saturating_sub(started)
}

pub fn busy_for(cpu: Duration) {
    let started = thread_cpu_time();
    while thread_cpu_time().saturating_sub(started) < cpu {
        std::hint::black_box(0u64);
    }
}
