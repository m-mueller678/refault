use crate::context::Context2;
use rand::Rng;

#[unsafe(no_mangle)]
unsafe extern "C" fn getrandom(buf: *mut u8, buflen: usize, _flags: u32) -> isize {
    Context2::with(|context| {
        if let Some(rng) = &mut *context.rng.borrow_mut() {
            unsafe {
                // ensure memory is initialized. Hopefully this is optimized out.
                buf.write_bytes(0, buflen);
                rng.fill(std::slice::from_raw_parts_mut(buf, buflen));
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
    Context2::with(|context| {
        if let Some(time) = context.time.get() {
            unsafe {
                tp.write(libc::timespec {
                    tv_sec: time.as_secs() as i64,
                    tv_nsec: time.subsec_nanos() as i64,
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
