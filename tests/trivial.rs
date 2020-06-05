use bitfield::size_of;
use lralloc_rs::{do_free, do_malloc};
use std::mem::MaybeUninit;
use core::ptr::null_mut;

#[test]
fn run() {
    unsafe {
        let o = (do_malloc(size_of::<Option<usize>>()) as *mut MaybeUninit<Option<usize>>);
        assert_ne!(o, null_mut());
        // println!("First allocation successful");
        *o = MaybeUninit::new(Some(15));
        let o = o as *mut Option<usize>;

        do_malloc(size_of::<[usize; 64]>());
        assert_ne!(o, null_mut());
        // println!("First allocation successful");

        do_free(o as *const Option<usize>);
    }
}
