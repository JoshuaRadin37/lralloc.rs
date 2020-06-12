#![allow(non_upper_case_globals)]

use crate::allocation_data::{get_heaps, Anchor, Descriptor, DescriptorNode, SuperBlockState};
use crate::mem_info::{align_addr, align_val, MAX_SZ, MAX_SZ_IDX, PAGE};
use std::ptr::null_mut;

use crate::size_classes::{get_size_class, init_size_class, SIZE_CLASSES};

use crate::page_map::S_PAGE_MAP;

use crate::alloc::{get_page_info_for_ptr, register_desc, unregister_desc, update_page_map};
use crate::bootstrap::{boostrap_reserve, bootstrap_cache, set_use_bootstrap, use_bootstrap};
use crate::pages::{page_alloc, page_free};
use crate::thread_cache::{fill_cache, flush_cache};
use atomic::{Atomic, Ordering};
use crossbeam::atomic::AtomicCell;
use spin::Mutex;
use std::ffi::c_void;
use std::fs::read;
use std::ops::Deref;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::thread;
use std::thread::ThreadId;

#[macro_use]
pub mod macros;

pub mod alloc;
pub mod allocation_data;
#[allow(unused)]
pub mod mem_info;
pub mod no_heap_mutex;
pub mod page_map;
pub mod pages;
pub mod size_classes;
pub mod thread_cache;
#[cfg(feature = "track_allocation")] pub mod info_dump;
pub mod single_access;

mod bootstrap;

pub mod auto_ptr;


mod apf;

#[macro_use]
extern crate bitfield;

static AVAILABLE_DESC: Mutex<DescriptorNode> = Mutex::new(DescriptorNode::new());

pub(crate) static mut MALLOC_INIT: AtomicBool = AtomicBool::new(false); // Only one can access init
pub(crate) static mut MALLOC_FINISH_INIT: AtomicBool = AtomicBool::new(false); // tells anyone who was stuck looping to continue
pub(crate) static mut MALLOC_SKIP: bool = false; // removes the need for atomicity once set to true, potentially increasing speed

pub static IN_CACHE: AtomicUsize = AtomicUsize::new(0);
pub static IN_BOOTSTRAP: AtomicUsize = AtomicUsize::new(0);

pub unsafe fn init_malloc() {
    init_size_class();

    S_PAGE_MAP.init();

    for idx in 0..MAX_SZ_IDX {
        let heap = get_heaps().get_heap_at_mut(idx);

        heap.partial_list.store(None, Ordering::Release);
        heap.size_class_index = idx;
    }

    boostrap_reserve.lock().init();

    MALLOC_SKIP = true;
    MALLOC_FINISH_INIT.store(true, Ordering::Release);
}

pub fn do_malloc(size: usize) -> *mut u8 {
    unsafe {
        if !MALLOC_SKIP {
            if !MALLOC_INIT.compare_and_swap(false, true, Ordering::AcqRel) {
                init_malloc();
            }
            while !MALLOC_FINISH_INIT.load(Ordering::Relaxed) {}
        }
    }

    if size > MAX_SZ {
        let pages = page_ceiling!(size);
        let desc = unsafe { &mut *Descriptor::alloc() };

        desc.proc_heap = null_mut();
        desc.block_size = pages as u32;
        desc.max_count = 1;
        desc.super_block = page_alloc(pages).expect("Should create");

        let mut anchor = Anchor::default();
        anchor.set_state(SuperBlockState::FULL);

        desc.anchor.store(anchor, Ordering::Release);

        register_desc(desc);
        let ptr = desc.super_block;
        // Log malloc with tuner
        return ptr;
    }

    let size_class_index = get_size_class(size);


    allocate_to_cache(size, size_class_index)
}

fn is_power_of_two(x: usize) -> bool {
    // https://stackoverflow.com/questions/3638431/determine-if-an-int-is-a-power-of-2-or-not-in-a-single-line
    (if x != 0 { true } else { false }) && (if (!(x & (x - 1))) != 0 { true } else { false })
}

pub fn do_aligned_alloc(align: usize, size: usize) -> *mut u8 {
    if !is_power_of_two(align) {
        return null_mut();
    }

    let mut size = align_val(size, align);

    unsafe {
        if !MALLOC_SKIP {
            if !MALLOC_INIT.compare_and_swap(false, true, Ordering::AcqRel) {
                init_malloc();
            }
            while !MALLOC_FINISH_INIT.load(Ordering::Relaxed) {}
        }
    }

    if size > PAGE {
        size = size.max(MAX_SZ + 1);

        let need_more_pages = align > PAGE;
        if need_more_pages {
            size += align;
        }

        let pages = page_ceiling!(size);

        let desc = unsafe { &mut *Descriptor::alloc() };

        let mut ptr = match page_alloc(pages) {
            Ok(ptr) => ptr,
            Err(_) => null_mut(),
        };

        desc.proc_heap = null_mut();
        desc.block_size = pages as u32;
        desc.max_count = 1;
        desc.super_block = match page_alloc(pages) {
            Ok(ptr) => ptr,
            Err(_) => null_mut(),
        };

        let mut anchor = Anchor::default();
        anchor.set_state(SuperBlockState::FULL);

        desc.anchor.store(anchor, Ordering::Release);

        register_desc(desc);

        if need_more_pages {
            ptr = align_addr(ptr as usize, align) as *mut u8;

            update_page_map(None, ptr, Some(desc), 0);
        }

        return ptr;
    }

    let size_class_index = get_size_class(size);

    /*

    The way this works pretty wild
    There is a global state of use_bootstrap

     */

    allocate_to_cache(size, size_class_index)
}

pub fn allocate_to_cache(size: usize, size_class_index: usize) -> *mut u8 {
    // Because of how rust creates thread locals, we have to assume the thread local does not exist yet
    // We also can't tell if a thread local exists without causing it to initialize, and when using
    // This as a global allocator, it ends up calling this function again. If not careful, we will create an
    // infinite recursion. As such, we must have a "bootstrap" bin that threads can use to initalize it's
    // own local bin

    // todo: remove the true
    //let id = thread::current();

    if use_bootstrap() {
        // This is a global state, and tells to allocate from the bootstrap cache
        /*
        unsafe {
            // Gets and fills the correct cache from the bootstrap
            let mut bootstrap_cache_guard = bootstrap_cache.lock();
            let cache = bootstrap_cache_guard.get_mut(size_class_index).unwrap();
            if cache.get_block_num() == 0 {
                fill_cache(size_class_index, cache);
            }

            cache.pop_block()
        }
         */
        #[cfg(debug_assertions)]
        unsafe {
            IN_BOOTSTRAP.fetch_add(size, Ordering::AcqRel);
        }
        unsafe { boostrap_reserve.lock().allocate(size) }
    } else {
        #[cfg(not(unix))]
        {
            set_use_bootstrap(true); // Sets the next allocation to use the bootstrap cache
                                     //WAIT_FOR_THREAD_INIT.store(Some(thread::current().id()));
            thread_cache::thread_init.with(|val| {
                // if not initalized, it goes back
                if !*val.borrow() {
                    // the default value of the val is false, which means that the thread cache has not been created yet
                    thread_cache::thread_cache.with(|tcache| {
                        // This causes another allocation, hopefully with bootstrap
                        let _tcache = tcache; // There is a theoretical bootstrap data race here, but because
                    }); // it repeatedly sets it false, eventually, it will allocate
                    *val.borrow_mut() = true; // Never has to repeat this code after this
                }
                set_use_bootstrap(false) // Turns off the bootstrap
            });
        }

        #[cfg(debug_assertions)]
        unsafe {
            IN_CACHE.fetch_add(size, Ordering::AcqRel);
        }
        // If we are able to reach this piece of code, we know that the thread local cache is initalized
        let ret = thread_cache::thread_cache.with(|tcache| {
            let cache = unsafe {
                (*tcache.get()).get_mut(size_class_index).unwrap() // Gets the correct bin based on size class index
            };

            if cache.get_block_num() == 0 {
                fill_cache(size_class_index, cache); // Fills the cache if necessary
                if cache.block_num == 0 {
                    panic!("Cache didn't fill");
                }
            }
            #[cfg(feature = "track_allocation")] {
                let ret = cache.pop_block();
                let size = get_allocation_size(ret as *const c_void).unwrap() as usize;
                crate::info_dump::log_malloc(size);
                ret
            }
            #[cfg(not(feature = "track_allocation"))]
            cache.pop_block() // Pops the block from the thread cache bin
        });

        #[cfg(unix)]
        {
            thread_cache::skip.with(|b| unsafe {
                if !*b.get() {
                    let mut skip = b.get();
                    *skip = true;
                    let _ = thread_cache::thread_init.with(|_| ());
                }
            })
        }

        ret
    }
}

pub fn do_realloc(ptr: *mut c_void, size: usize) -> *mut c_void {
    let new_size_class = get_size_class(size);
    let old_size = match get_allocation_size(ptr) {
        Ok(size) => size as usize,
        Err(_) => {
            return null_mut();
        }
    };
    let old_size_class = get_size_class(old_size);
    if old_size_class == new_size_class {
        return ptr;
    }

    let ret = do_malloc(size) as *mut c_void;
    unsafe {
        libc::memcpy(ret, ptr, old_size);
    }
    do_free(ptr);
    ret
}

pub fn get_allocation_size(ptr: *const c_void) -> Result<u32, ()> {
    let info = get_page_info_for_ptr(ptr);
    let desc = unsafe { &*info.get_desc().ok_or(())? };

    Ok(desc.block_size)
}

#[allow(unused)]
fn do_malloc_aligned_from_bootstrap(align: usize, size: usize) -> *mut u8 {
    if !is_power_of_two(align) {
        return null_mut();
    }

    let mut size = align_val(size, align);

    unsafe {
        if !MALLOC_INIT.compare_and_swap(false, true, Ordering::AcqRel) {
            init_malloc();
        }
        while !MALLOC_FINISH_INIT.load(Ordering::Relaxed) {}
    }

    if size > PAGE {
        size = size.max(MAX_SZ + 1);

        let need_more_pages = align > PAGE;
        if need_more_pages {
            size += align;
        }

        let pages = page_ceiling!(size);

        let desc = unsafe { &mut *Descriptor::alloc() };

        let mut ptr = page_alloc(pages).expect("Error getting pages for aligned allocation");

        desc.proc_heap = null_mut();
        desc.block_size = pages as u32;
        desc.max_count = 1;
        desc.super_block = page_alloc(pages).expect("Should create");

        let mut anchor = Anchor::default();
        anchor.set_state(SuperBlockState::FULL);

        desc.anchor.store(anchor, Ordering::Release);

        register_desc(desc);

        if need_more_pages {
            ptr = align_addr(ptr as usize, align) as *mut u8;

            update_page_map(None, ptr, Some(desc), 0);
        }

        return ptr;
    }

    let size_class_index = get_size_class(size);

    unsafe {
        let mut bootstrap_cache_guard = bootstrap_cache.lock();
        let cache = bootstrap_cache_guard.get_mut(size_class_index).unwrap();
        if cache.get_block_num() == 0 {
            fill_cache(size_class_index, cache);
        }

        cache.pop_block()
    }
}

pub fn do_free<T : ?Sized>(ptr: *const T) {
    let info = get_page_info_for_ptr(ptr);
    let desc = unsafe {
        &mut *match info.get_desc() {
            Some(d) => d,
            None => {
                // #[cfg(debug_assertions)]
                // println!("Free failed at {:?}", ptr);
                return; // todo: Band-aid fix
                        // panic!("Descriptor not found for the pointer {:x?} with page info {:?}", ptr, info);
            }
        }
    };

    // #[cfg(debug_assertions)]
    // println!("Free will succeed at {:?}", ptr);

    let size_class_index = info.get_size_class_index();
    match size_class_index {
        None | Some(0) => {
            let super_block = desc.super_block;
            // unregister
            unregister_desc(None, super_block);

            // if large allocation
            if ptr as *const u8 != super_block as *const u8 {
                unregister_desc(None, ptr as *mut u8)
            }

            // free the super block
            page_free(super_block);

            // retire the descriptor
            desc.retire();
        }
        Some(size_class_index) => {
            let force_bootstrap = unsafe { boostrap_reserve.lock().ptr_in_bootstrap(ptr) }
                || use_bootstrap()
                || (!cfg!(unix)
                    && match thread_cache::thread_init.try_with(|_| {}) {
                        Ok(_) => false,
                        Err(_) => true,
                    });
            // todo: remove true
            #[cfg(feature = "track_allocation")] {
                crate::info_dump::log_free(get_allocation_size(ptr as *const c_void).unwrap() as usize);
            }
            if force_bootstrap {
                unsafe {
                    /*
                    let mut bootstrap_cache_guard = bootstrap_cache.lock();
                    let cache = bootstrap_cache_guard.get_mut(size_class_index).unwrap();
                    let sc = &SIZE_CLASSES[size_class_index];

                    if cache.get_block_num() >= sc.cache_block_num {
                        flush_cache(size_class_index, cache);
                    }

                    cache.push_block(ptr as *mut u8);

                     */
                }
            } else {
                #[cfg(not(unix))]
                {
                    set_use_bootstrap(true);
                    thread_cache::thread_init.with(|val| {
                        if !*val.borrow() {
                            thread_cache::thread_cache.with(|tcache| {
                                let _tcache = tcache;
                            });
                            *val.borrow_mut() = true;
                        }
                        set_use_bootstrap(false)
                    });
                }
                thread_cache::thread_cache
                    .try_with(|tcache| {
                        let cache = unsafe { (*tcache.get()).get_mut(size_class_index).unwrap() };
                        let sc = unsafe { &SIZE_CLASSES[size_class_index] };
                        /*
                        if sc.block_num == 0 {
                            unsafe {
                                let mut guard = bootstrap_cache.lock();
                                let cache = guard.get_mut(size_class_index).unwrap();


                                if cache.get_block_num() >= sc.cache_block_num {
                                    flush_cache(size_class_index, cache);
                                }

                                return cache.push_block(ptr as *mut u8);
                            }
                        }

                         */

                        if cache.get_block_num() >= sc.cache_block_num {
                            flush_cache(size_class_index, cache);
                        }

                        return cache.push_block(ptr as *mut u8);
                    })
                    .expect("Freeing to cache failed");
            }
        }
    }
}


#[macro_export]
macro_rules! dump_info {

    () => {
        #[cfg(feature = "track_allocation")] println!("{:?}", crate::info_dump::get_info_dump())
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::allocation_data::get_heaps;
    use bitfield::size_of;
    use core::mem::MaybeUninit;

    #[test]
    fn heaps_valid() {
        let heap = get_heaps();
        let _p_heap = heap.get_heap_at_mut(0);
    }

    #[test]
    fn malloc_and_free() {
        let ptr =
            unsafe { &mut *(super::do_malloc(size_of::<usize>()) as *mut MaybeUninit<usize>) };
        *ptr = MaybeUninit::new(8);
        assert_eq!(
            &unsafe { *(ptr as *const MaybeUninit<usize> as *const u8) },
            &8
        ); // should be trivial
        do_free(ptr as *mut MaybeUninit<usize>);
    }

    #[test]
    fn malloc_and_free_large() {
        let ptr = super::do_malloc(MAX_SZ * 2);
        do_free(ptr);
    }

    #[test]
    fn cache_pop_no_fail() {
        const size_class: usize = 16;
        unsafe {
            if !MALLOC_INIT.compare_and_swap(false, true, Ordering::AcqRel) {
                init_malloc();
            }
            while !MALLOC_FINISH_INIT.load(Ordering::Relaxed) {}
        }

        let sc = unsafe { &SIZE_CLASSES[size_class] };
        let total_blocks = sc.block_num;
        let block_size = sc.block_size;

        let test_blocks = total_blocks * 3 / 2;
        let null: *mut u8 = null_mut();
        let mut ptrs = vec![];
        for _ in 0..test_blocks {
            let ptr = do_malloc(block_size as usize);
            unsafe {
                *ptr = b'1';
            }
            assert_ne!(
                ptr, null,
                "Did not successfully get a pointer from the cache"
            );
            ptrs.push(ptr);
        }
        for ptr in ptrs {
            do_free(ptr);
        }
    }

    #[test]
    fn zero_size_malloc() {
        let v = do_malloc(0);
        assert_ne!(v, null_mut());
        assert_eq!(get_allocation_size(v as *const c_void).expect("Zero Sized Allocation should act as an 8 byte allocation"), 8);
        do_free(v);
    }

}

#[cfg(test)]
mod track_allocation_tests {
    use crate::auto_ptr::AutoPtr;

    #[cfg(feature = "track_allocation")]
    #[test]
    fn info_dump_one_thread() {
        let first_ptrs = (0..10)
            .into_iter()
            .map(|_| AutoPtr::new(0usize))
            .collect::<Vec<_>>();

        dump_info!();

        {
            let first_ptrs = (0..10)
                .into_iter()
                .map(|_| AutoPtr::new([0usize; 16]))
                .collect::<Vec<_>>();
            dump_info!();
        }


        dump_info!();
    }
}
