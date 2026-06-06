extern crate alloc;

use crate::gdt;
use core::arch::naked_asm;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use spin::Mutex;
use x86_64::instructions::tables;
use x86_64::registers::control::{Cr3, Cr3Flags};
use x86_64::structures::paging::PhysFrame;
use x86_64::{PhysAddr, VirtAddr};

const KERNEL_STACK_PAGES: usize = 16;
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
const MAX_KERNEL_STACKS: usize = MAX_PROCESSES;
static mut KERNEL_STACK_POOL: [u8; KERNEL_STACK_SIZE * MAX_KERNEL_STACKS] =
    [0; KERNEL_STACK_SIZE * MAX_KERNEL_STACKS];
static mut NEXT_KERNEL_STACK: usize = 0;

pub fn allocate_kernel_stack() -> VirtAddr {
    unsafe {
        let idx = NEXT_KERNEL_STACK;
        if idx >= MAX_KERNEL_STACKS {
            panic!("kernel stack pool exhausted");
        }
        NEXT_KERNEL_STACK = idx + 1;
        let base = (core::ptr::addr_of!(KERNEL_STACK_POOL) as u64)
            + (idx as u64 * KERNEL_STACK_SIZE as u64);
        VirtAddr::new(base + KERNEL_STACK_SIZE as u64)
    }
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

#[unsafe(naked)]
pub extern "C" fn return_to_user() {
    naked_asm!("iretq")
}

#[unsafe(naked)]
pub extern "C" fn context_switch(_old_rsp_out: *mut u64, _new_rsp: u64) {
    naked_asm!(
        "push r15",
        "push r14",
        "push r13",
        "push r12",
        "push rbp",
        "push rbx",
        "mov [rdi], rsp",
        "mov rsp, rsi",
        "pop rbx",
        "pop rbp",
        "pop r12",
        "pop r13",
        "pop r14",
        "pop r15",
        "ret",
    )
}

pub static mut KERNEL_RSP: u64 = 0;
pub static CURRENT_PROCESS: AtomicUsize = AtomicUsize::new(0);

pub unsafe fn switch_to(old_idx: usize, new_idx: usize) {
    let new_cr3: x86_64::PhysAddr;
    let new_kernel_stack_top: VirtAddr;
    let new_rsp: u64;
    let old_rsp_slot: *mut u64;

    {
        let table = PROCESSES.lock();
        let old_proc = table.get(old_idx).expect("switch_to: invalid old_idx");
        let new_proc = table.get(new_idx).expect("switch_to: invalid new_idx");
        new_cr3 = new_proc.cr3;
        new_kernel_stack_top = new_proc.kernel_stack_top;
        new_rsp = new_proc.kernel_rsp.as_u64();
        old_rsp_slot = core::ptr::addr_of!(old_proc.kernel_rsp) as *mut u64;
    }

    CURRENT_PROCESS.store(new_idx, Ordering::SeqCst);

    KERNEL_RSP = new_kernel_stack_top.as_u64();
    gdt::set_kernel_rsp0(new_kernel_stack_top);
    Cr3::write(PhysFrame::containing_address(new_cr3), Cr3Flags::empty());

    context_switch(old_rsp_slot, new_rsp);
}

pub unsafe fn setup_first_dispatch(
    kernel_stack_top: VirtAddr,
    user_rip: u64,
    user_cs: u64,
    user_rflags: u64,
    user_rsp: u64,
    user_ss: u64,
) -> VirtAddr {
    let mut p = kernel_stack_top.as_u64();

    p -= 8;
    *(p as *mut u64) = user_ss;
    p -= 8;
    *(p as *mut u64) = user_rsp;
    p -= 8;
    *(p as *mut u64) = user_rflags;
    p -= 8;
    *(p as *mut u64) = user_cs;
    p -= 8;
    *(p as *mut u64) = user_rip;
    p -= 8;
    *(p as *mut u64) = return_to_user as u64;
    p -= 8;
    *(p as *mut u64) = 0;
    p -= 8;
    *(p as *mut u64) = 0;
    p -= 8;
    *(p as *mut u64) = 0;
    p -= 8;
    *(p as *mut u64) = 0;
    p -= 8;
    *(p as *mut u64) = 0;
    p -= 8;
    *(p as *mut u64) = 0;

    VirtAddr::new(p)
}
