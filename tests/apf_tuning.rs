extern crate apfmalloc_lib;

use apfmalloc_lib::ptr::auto_ptr::AutoPtr;
use apfmalloc_lib::{do_aligned_alloc, do_free};
use std::alloc::{GlobalAlloc, Layout};
use std::thread;

struct Apf;

unsafe impl GlobalAlloc for Apf {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        do_aligned_alloc(layout.align(), layout.size())
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        do_free(ptr);
    }
}

#[global_allocator]
static ALLOCATOR: Apf = Apf;

#[test]
fn test_apf_tuning() {
    let mut vec = vec![];
    let thread_count = 1;

    for _i in 0..thread_count {
        vec.push(thread::spawn(move || {
            //println!("Thread {}", &i);
            for _j in 2..10 {
                let mut ptrs = vec![];
                for _p in 0..1000 * _j {
                    ptrs.push(AutoPtr::new(_i * _p));
                }
            }

            _i
        }));
    }

    for join_handle in vec {
        println!("{}", join_handle.join().unwrap());
    }
}
