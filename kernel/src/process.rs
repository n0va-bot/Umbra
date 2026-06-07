extern crate alloc;

use core::arch::naked_asm;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use spin::Mutex;
use x86_64::registers::control::{Cr3, Cr3Flags};
use x86_64::structures::paging::{
    FrameAllocator, Mapper, Page, PageTableFlags, PhysFrame, Size4KiB,
};
use x86_64::{PhysAddr, VirtAddr};

const KERNEL_STACK_PAGES: usize = 16;
pub const MAX_PROCESSES: usize = 16;

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

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct InterruptFrame {
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

impl Default for InterruptFrame {
    fn default() -> Self {
        Self {
            rip: 0,
            cs: 0,
            rflags: 0,
            rsp: 0,
            ss: 0,
        }
    }
}

#[derive(Debug)]
pub struct Process {
    pub pid: Pid,
    pub state: State,
    pub cr3: PhysAddr,
    pub kernel_stack_top: VirtAddr,
    pub kernel_rsp: VirtAddr,
    pub saved: SavedRegs,
    pub interrupt_frame: InterruptFrame,
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

    pub fn remove(&mut self, index: usize) -> Option<Process> {
        self.slots.get_mut(index).and_then(|slot| slot.take())
    }

    pub fn iter(&self) -> impl Iterator<Item = (usize, &Process)> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.as_ref().map(|p| (i, p)))
    }
}

pub static PROCESSES: Mutex<ProcessTable> = Mutex::new(ProcessTable::new());

pub fn schedule(after_idx: usize) -> Option<usize> {
    let table = PROCESSES.lock();
    for i in 1..=MAX_PROCESSES {
        let idx = (after_idx + i) % MAX_PROCESSES;
        if let Some(proc) = table.get(idx) {
            if proc.state == State::Ready {
                return Some(idx);
            }
        }
    }
    None
}

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
    let new_cr3: PhysAddr;
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

    unsafe {
        KERNEL_RSP = new_kernel_stack_top.as_u64();
    }
    crate::gdt::set_kernel_rsp0(new_kernel_stack_top);
    unsafe {
        Cr3::write(PhysFrame::containing_address(new_cr3), Cr3Flags::empty());
    }

    context_switch(old_rsp_slot, new_rsp);
}

/// Preemptive context switch from interrupt handler.
/// The interrupt frame is on the current kernel stack at `interrupt_rsp`.
/// Returns the new kernel RSP to load, or 0 if no switch needed.
pub unsafe fn preempt_switch(interrupt_rsp: u64) -> u64 {
    let current = CURRENT_PROCESS.load(Ordering::SeqCst);
    if current == 0 {
        return 0;
    }

    let next_idx = {
        let table = PROCESSES.lock();
        let mut found = None;
        for i in 1..=MAX_PROCESSES {
            let idx = (current + i) % MAX_PROCESSES;
            if let Some(proc) = table.get(idx) {
                if proc.state == State::Ready && proc.interrupt_frame.rip != 0 {
                    found = Some(idx);
                    break;
                }
            }
        }
        found
    };

    let Some(new_idx) = next_idx else {
        return 0;
    };

    if new_idx == current {
        return 0;
    }

    let (new_cr3, new_kernel_stack_top, new_rsp, old_proc_ptr, new_proc_ptr) = {
        let table = PROCESSES.lock();
        let old_proc = table.get(current).expect("preempt_switch: invalid current");
        let new_proc = table.get(new_idx).expect("preempt_switch: invalid new_idx");
        (
            new_proc.cr3,
            new_proc.kernel_stack_top,
            new_proc.kernel_rsp.as_u64(),
            core::ptr::addr_of!(*old_proc) as *mut Process,
            core::ptr::addr_of!(*new_proc) as *mut Process,
        )
    };

    let old_frame_ptr = (interrupt_rsp + 48) as *const InterruptFrame;
    unsafe {
        (*old_proc_ptr).interrupt_frame = *old_frame_ptr;
    }

    CURRENT_PROCESS.store(new_idx, Ordering::SeqCst);

    unsafe {
        KERNEL_RSP = new_kernel_stack_top.as_u64();
    }
    crate::gdt::set_kernel_rsp0(new_kernel_stack_top);
    unsafe {
        Cr3::write(PhysFrame::containing_address(new_cr3), Cr3Flags::empty());
    }

    let new_frame_ptr = (new_rsp + 48) as *mut InterruptFrame;
    unsafe {
        *new_frame_ptr = (*new_proc_ptr).interrupt_frame;
    }

    new_rsp
}

/// Check if a process has been preempted before (has a valid interrupt frame).
pub fn has_valid_interrupt_frame(idx: usize) -> bool {
    let table = PROCESSES.lock();
    if let Some(proc) = table.get(idx) {
        proc.interrupt_frame.rip != 0
    } else {
        false
    }
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

    unsafe {
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
        *(p as *mut u64) = return_to_user as *const () as u64;
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
    }

    VirtAddr::new(p)
}

const USER_STACK_TOP: u64 = 0x5555_0000_0000 + 4096;

pub fn spawn(elf_bytes: &[u8], frame_allocator: &mut impl FrameAllocator<Size4KiB>) -> usize {
    let new_cr3 = unsafe { crate::memory::clone_kernel_pml4(frame_allocator) };

    let mut mapper = unsafe { crate::memory::create_mapper_for_pml4(new_cr3) };

    let entry_point = crate::elf_loader::load_elf_into(elf_bytes, &mut mapper, frame_allocator);

    let stack_page = Page::containing_address(VirtAddr::new(0x5555_0000_0000));
    let stack_frame = frame_allocator
        .allocate_frame()
        .expect("out of frames for user stack");
    let flags =
        PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE;
    unsafe {
        mapper
            .map_to(stack_page, stack_frame, flags, frame_allocator)
            .expect("map_to user stack failed")
            .flush();
    }

    let kernel_stack_top = allocate_kernel_stack();

    let user_cs = crate::gdt::get_user_code_selector().0 as u64 | 3;
    let user_ss = crate::gdt::get_user_data_selector().0 as u64 | 3;
    const RFLAGS_IF: u64 = 1 << 9;

    let kernel_rsp = unsafe {
        setup_first_dispatch(
            kernel_stack_top,
            entry_point,
            user_cs,
            RFLAGS_IF,
            USER_STACK_TOP,
            user_ss,
        )
    };

    let pid = Pid::alloc();
    let process = Process {
        pid,
        state: State::Ready,
        cr3: new_cr3,
        kernel_stack_top,
        kernel_rsp,
        saved: SavedRegs::default(),
        interrupt_frame: InterruptFrame::default(),
    };

    let index = PROCESSES.lock().insert(process);
    crate::serial_println!(
        "[process] spawned PID {} at index {} (CR3={:#X}, entry={:#X})",
        pid.0,
        index,
        new_cr3.as_u64(),
        entry_point
    );
    index
}

pub fn exit(index: usize) {
    let mut table = PROCESSES.lock();
    if let Some(proc) = table.get_mut(index) {
        crate::serial_println!("[process] PID {} exited", proc.pid.0);
        proc.state = State::Exited;
    }
}

/// Validate that a user-space pointer is accessible in the current process.
/// Returns the physical address if valid, None otherwise.
pub fn validate_user_ptr(vaddr: VirtAddr) -> Option<PhysAddr> {
    let current = CURRENT_PROCESS.load(Ordering::SeqCst);
    if current == 0 {
        return None;
    }

    let table = PROCESSES.lock();
    let proc = table.get(current)?;
    let cr3 = proc.cr3;
    drop(table);

    let offset = crate::memory::get_phys_mem_offset();

    let table_indexes = [
        vaddr.p4_index(),
        vaddr.p3_index(),
        vaddr.p2_index(),
        vaddr.p1_index(),
    ];
    let mut frame = PhysFrame::containing_address(cr3);

    for &index in &table_indexes {
        let virt = offset + frame.start_address().as_u64();
        let table_ptr: *const x86_64::structures::paging::PageTable = virt.as_ptr();
        let table = unsafe { &*table_ptr };
        let entry = &table[index];
        frame = match entry.frame() {
            Ok(f) => f,
            Err(x86_64::structures::paging::page_table::FrameError::FrameNotPresent) => return None,
            Err(x86_64::structures::paging::page_table::FrameError::HugeFrame) => return None,
        };
        if !entry.flags().contains(x86_64::structures::paging::PageTableFlags::USER_ACCESSIBLE) {
            return None;
        }
    }

    Some(frame.start_address() + u64::from(vaddr.page_offset()))
}
