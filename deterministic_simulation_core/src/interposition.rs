use std::time::Duration;
use crate::context::{Context, CONTEXT};

#[unsafe(no_mangle)]
unsafe extern "C" fn getrandom(buf: *mut u8, buflen: usize, _flags: u32) -> isize {
    for i in 0 .. buflen {
        *buf.wrapping_add(i) = 0;
    }
    buflen as isize
}

#[unsafe(no_mangle)]
unsafe extern "C" fn clock_gettime(
    _clockid: libc::clockid_t,
    tp: *mut libc::timespec,
) -> libc::c_int {
    let mut binding = CONTEXT.lock().unwrap();
    let context: &mut Context = binding.as_mut().unwrap();
    let execution_duration = context.executor.time_scheduler.lock().unwrap().elapsed();
    
    let result_duration = execution_duration + Duration::from_millis(1750363615882u64);
    tp.write(libc::timespec {
        tv_sec: result_duration.as_secs() as i64,
        tv_nsec: result_duration.subsec_nanos() as i64,
    });
    0
}
