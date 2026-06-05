extern crate alloc;

use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use spin::Mutex;
use x86_64::{PhysAddr, VirtAddr};

pub const KERNEL_STACK_PAGES: usize = 16;
const MAX_PROCESSES: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Ready,
    Running,
    Blocked,
    Exited,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Pid(pub u64);

impl Pid {
    pub fn alloc() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        Pid(id)
    }
}

#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
pub struct SavedRegs {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub rbp: u64,
    pub rbx: u64,
}

#[derive(Debug)]
pub struct Process {
    pub pid: Pid,
    pub state: State,
    pub cr3: PhysAddr,
    pub kernel_stack_top: VirtAddr,
    pub kernel_rsp: VirtAddr,
    pub saved: SavedRegs,
}

pub const KERNEL_STACK_SIZE: usize = 4096 * KERNEL_STACK_PAGES;

pub fn allocate_kernel_stack() -> VirtAddr {
    let layout = alloc::alloc::Layout::from_size_align(KERNEL_STACK_SIZE, 4096)
        .expect("kernel stack layout");
    let base = unsafe { alloc::alloc::alloc_zeroed(layout) } as u64;

    if base == 0 {
        panic!("kernel stack allocation failed (out of memory)");
    }

    static COUNT: AtomicUsize = AtomicUsize::new(0);
    let _ = COUNT.fetch_add(1, Ordering::Relaxed);

    VirtAddr::new(base + KERNEL_STACK_SIZE as u64)
}

pub struct ProcessTable {
    slots: [Option<Process>; MAX_PROCESSES],
    current: Option<usize>,
}

impl ProcessTable {
    pub const fn new() -> Self {
        const NONE: Option<Process> = None;
        ProcessTable {
            slots: [NONE; MAX_PROCESSES],
            current: None,
        }
    }

    pub fn current(&self) -> Option<&Process> {
        self.current.and_then(|i| self.slots[i].as_ref())
    }

    pub fn current_mut(&mut self) -> Option<&mut Process> {
        self.current.and_then(|i| self.slots[i].as_mut())
    }

    pub fn current_index(&self) -> Option<usize> {
        self.current
    }

    pub fn set_current(&mut self, index: usize) {
        self.current = Some(index);
    }

    pub fn insert(&mut self, process: Process) -> usize {
        for (idx, slot) in self.slots.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(process);
                return idx;
            }
        }
        panic!(
            "process table is full ({}/{} processes)",
            self.slots.len(),
            MAX_PROCESSES
        );
    }

    pub fn get(&self, index: usize) -> Option<&Process> {
        self.slots.get(index).and_then(|s| s.as_ref())
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut Process> {
        self.slots.get_mut(index).and_then(|s| s.as_mut())
    }

    pub fn iter(&self) -> impl Iterator<Item = (usize, &Process)> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.as_ref().map(|p| (i, p)))
    }
}

pub static PROCESSES: Mutex<ProcessTable> = Mutex::new(ProcessTable::new());
