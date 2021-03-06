use crate::mem_info::MAX_SZ_IDX;
use spin::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone)]
pub struct InfoDump {
    total_allocated_from_vm: usize,
    current_allocated_from_vm: usize,
    current_mem_allocated: usize,
    total_allocs: usize,
    total_frees: usize,
}

static INFO_DUMP: Mutex<InfoDump> = Mutex::new(InfoDump {
    total_allocated_from_vm: 0,
    current_allocated_from_vm: 0,
    current_mem_allocated: 0,
    total_allocs: 0,
    total_frees: 0,
});

pub fn increase_allocated_from_vm(change: usize) {
    let mut info_dump = INFO_DUMP.lock();
    info_dump.current_allocated_from_vm += change;
    info_dump.total_allocated_from_vm += change;
}

pub fn decrease_allocated_from_vm(change: usize) {
    let mut info_dump = INFO_DUMP.lock();
    assert!(info_dump.current_allocated_from_vm >= change);
    info_dump.current_allocated_from_vm -= change;
}

pub fn log_malloc(size: usize) {
    let mut info_dump = INFO_DUMP.lock();
    info_dump.current_mem_allocated += size;
    info_dump.total_allocs += 1;
}

pub fn log_free(size: usize) {
    let mut info_dump = INFO_DUMP.lock();
    info_dump.current_mem_allocated -= size;
    info_dump.total_frees += 1;
}

pub fn get_info_dump() -> InfoDump {
    static skip: AtomicBool = AtomicBool::new(false);
    let guard = INFO_DUMP.lock();
    guard.clone()
}

pub fn print_info_dump() {
    static skip: AtomicBool = AtomicBool::new(false);
    if !skip.compare_and_swap(false, true, Ordering::Acquire) {
        println!("{:?}", get_info_dump());
        skip.store(false, Ordering::Relaxed);
    }
}
