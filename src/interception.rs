use crate::context::with_context_option;
use rand::RngCore;
use std::time::Duration;

#[unsafe(no_mangle)]
unsafe extern "C" fn getrandom(buf: *mut u8, buflen: usize, _flags: u32) -> isize {
    with_context_option(|context| {
        if let Some(context) = context {
            unsafe {
                for i in 0..buflen {
                    *buf.wrapping_add(i) = context.random_generator.next_u32() as u8;
                }
                buflen as isize
            }
        } else {
            // Provide normal random number generation when not within a simulation
            // This section has been compied from Madsim: https://github.com/madsim-rs/madsim/blob/main/madsim/src/sim/rand.rs
            lazy_static::lazy_static! {
                static ref GETRANDOM: unsafe extern "C" fn(buf: *mut u8, buflen: usize, flags: u32) -> isize = unsafe {
                    let ptr = libc::dlsym(libc::RTLD_NEXT, c"getrandom".as_ptr() as _);
                    assert!(!ptr.is_null());
                    std::mem::transmute(ptr)
                };
            }
            unsafe { GETRANDOM(buf, buflen, _flags) }
        }
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn clock_gettime(
    _clockid: libc::clockid_t,
    tp: *mut libc::timespec,
) -> libc::c_int {
    with_context_option(|context| {
        if let Some(context) = context {
            unsafe {
                let execution_duration = context.executor.time_scheduler.lock().unwrap().elapsed();

                let result_duration =
                    execution_duration + Duration::from_millis(context.simulation_start_time);
                tp.write(libc::timespec {
                    tv_sec: result_duration.as_secs() as i64,
                    tv_nsec: result_duration.subsec_nanos() as i64,
                });
                0
            }
        } else {
            // Provide normal time when not within a simulation
            // This section has been compied from Madsim: https://github.com/madsim-rs/madsim/blob/main/madsim/src/sim/time/system_time.rs
            lazy_static::lazy_static! {
                static ref CLOCK_GETTIME: unsafe extern "C" fn(
                    clockid: libc::clockid_t,
                    tp: *mut libc::timespec,
                ) -> libc::c_int = unsafe {
                    let ptr = libc::dlsym(libc::RTLD_NEXT, c"clock_gettime".as_ptr() as _);
                    assert!(!ptr.is_null());
                    std::mem::transmute(ptr)
                };
            }
            unsafe { CLOCK_GETTIME(_clockid, tp) }
        }
    })
}
