use crate::mem_info::MAX_SZ_IDX;
use crate::pages::external_mem_reservation::{SegAllocator, Segment, SEGMENT_ALLOCATOR};
use crate::thread_cache::ThreadCacheBin;
use spin::Mutex;
use std::process::exit;
use std::ptr::null_mut;

#[allow(unused)]
pub static mut bootstrap_cache: Mutex<[ThreadCacheBin; MAX_SZ_IDX]> =
    Mutex::new([ThreadCacheBin::new(); MAX_SZ_IDX]);

static _use_bootstrap: Mutex<bool> = Mutex::new(false);

pub fn use_bootstrap() -> bool {
    *_use_bootstrap.lock()
}

/// When using the bootstrap, all threads allocate from a single source location which does not require any heap allocation itself
/// This is useful for systems that require heap space to allocate static variables of types that implement copy.
#[allow(unused)]
pub fn set_use_bootstrap(val: bool) {
    *_use_bootstrap.lock() = val;
}

pub struct BootstrapReserve {
    mem: Option<Segment>,
    next: *mut u8,
    avail: usize,
    max: usize,
}

impl BootstrapReserve {
    pub const fn new(size: usize) -> Self {
        Self {
            mem: None,
            next: null_mut(),
            avail: 0,
            max: size,
        }
    }

    pub fn init(&mut self) {
        match &mut self.mem {
            None => {
                std::mem::replace(
                    &mut self.mem,
                    Some(
                        SEGMENT_ALLOCATOR
                            .allocate(self.max)
                            .unwrap_or_else(|_| exit(-1)),
                    ),
                );
                // self.mem = Some(SEGMENT_ALLOCATOR.allocate(self.avail).unwrap_or_else(|_| exit(-1)));
                self.next = self.mem.as_ref().unwrap().get_ptr() as *mut u8;
                self.avail = self.max;
            }
            Some(seg) => {
                *seg = SEGMENT_ALLOCATOR
                    .allocate(self.max)
                    .unwrap_or_else(|_| exit(-1));
                self.next = seg.get_ptr() as *mut u8;
                self.avail = self.max;
            }
        }
    }

    pub unsafe fn allocate(&mut self, size: usize) -> *mut u8 {
        if size > self.avail {
            return null_mut();
        }

        let ret = self.next;
        self.next = self.next.offset(size as isize);
        self.avail -= size;
        ret
    }

    pub fn ptr_in_bootstrap<T: ?Sized>(&self, ptr: *const T) -> bool {
        if let Some(segment) = &self.mem {
            let start = segment.get_ptr() as usize;
            let end = start + self.max;
            ptr as *const u8 as usize >= start && (ptr as *const u8 as usize) < end
        } else {
            panic!("No bootstrap memory");
        }
    }
}

#[allow(unused)]
const KB: usize = 1028;
#[allow(unused)]
const MB: usize = 1028 * KB;

pub static mut bootstrap_reserve: Mutex<BootstrapReserve> =
    Mutex::new(BootstrapReserve::new(128 * KB));
