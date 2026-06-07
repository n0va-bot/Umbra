extern crate alloc;

use core::arch::naked_asm;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use spin::Mutex;
use x86_64::registers::control::{Cr3, Cr3Flags};
use x86_64::structures::paging::{
    FrameAllocator, Mapper, Page, PageTableFlags, PhysFrame, Size4KiB,
};
use x86_64::{PhysAddr, VirtAddr, structures::paging::PageTable};

use crate::memory::BootInfoFrameAllocator;

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
    /// Index into the static [`KERNEL_STACK_POOL`] backing this process's
    /// kernel stack. Required to return the slot to the pool on teardown.
    pub kernel_stack_slot: usize,
    pub kernel_rsp: VirtAddr,
    pub saved: SavedRegs,
    pub interrupt_frame: InterruptFrame,
}

pub const KERNEL_STACK_SIZE: usize = 4096 * KERNEL_STACK_PAGES;
const MAX_KERNEL_STACKS: usize = MAX_PROCESSES;
static mut KERNEL_STACK_POOL: [u8; KERNEL_STACK_SIZE * MAX_KERNEL_STACKS] =
    [0; KERNEL_STACK_SIZE * MAX_KERNEL_STACKS];

/// Bit `i` set means kernel stack slot `i` is in use.
///
/// A bitmap (rather than a forward counter) lets us hand slots back to the
/// pool on process teardown. Only ever touched from the kernel process's
/// scheduler loop, which is single-threaded by construction.
static mut KERNEL_STACK_BITMAP: u16 = 0;

/// Allocate a kernel stack from the static pool.
///
/// Returns `(slot_index, stack_top_virt_addr)`. The caller is responsible
/// for storing `slot_index` in the [`Process`] so it can be passed to
/// [`free_kernel_stack`] on teardown.
pub fn allocate_kernel_stack() -> (usize, VirtAddr) {
    unsafe {
        for slot in 0..MAX_KERNEL_STACKS {
            if KERNEL_STACK_BITMAP & (1u16 << slot) == 0 {
                KERNEL_STACK_BITMAP |= 1u16 << slot;
                let base = (core::ptr::addr_of!(KERNEL_STACK_POOL) as u64)
                    + (slot as u64 * KERNEL_STACK_SIZE as u64);
                return (slot, VirtAddr::new(base + KERNEL_STACK_SIZE as u64));
            }
        }
    }
    panic!("kernel stack pool exhausted");
}

/// Return a kernel stack slot to the pool. The slot must currently be in
/// use (i.e. allocated and not yet freed).
pub fn free_kernel_stack(slot: usize) {
    assert!(
        slot < MAX_KERNEL_STACKS,
        "free_kernel_stack: slot {slot} out of range"
    );
    unsafe {
        assert!(
            KERNEL_STACK_BITMAP & (1u16 << slot) != 0,
            "free_kernel_stack: slot {slot} already free"
        );
        KERNEL_STACK_BITMAP &= !(1u16 << slot);
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

    let (kernel_stack_slot, kernel_stack_top) = allocate_kernel_stack();

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
        kernel_stack_slot,
        kernel_rsp,
        saved: SavedRegs::default(),
        interrupt_frame: InterruptFrame::default(),
    };

    let index = PROCESSES.lock().insert(process);
    crate::serial_println!(
        "[process] spawned PID {} at index {} (CR3={:#X}, entry={:#X}, kstack_slot={})",
        pid.0,
        index,
        new_cr3.as_u64(),
        entry_point,
        kernel_stack_slot
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

pub fn teardown(index: usize, frame_allocator: &mut BootInfoFrameAllocator) {
    if index == 0 {
        return;
    }

    let (cr3, slot) = {
        let mut table = PROCESSES.lock();
        let proc = match table.get_mut(index) {
            Some(p) => p,
            None => return,
        };
        let cr3 = proc.cr3;
        let slot = proc.kernel_stack_slot;
        table.remove(index);
        (cr3, slot)
    };

    deallocate_user_tables(PhysFrame::containing_address(cr3), frame_allocator);

    free_kernel_stack(slot);
    crate::serial_println!(
        "[process] tore down slot {} (kernel stack slot {}, PML4 {:#X})",
        index,
        slot,
        cr3.as_u64()
    );
}

pub fn teardown_exited(frame_allocator: &mut BootInfoFrameAllocator) {
    let mut to_teardown: [usize; MAX_PROCESSES] = [0; MAX_PROCESSES];
    let mut count = 0;

    {
        let table = PROCESSES.lock();
        for i in 1..MAX_PROCESSES {
            if let Some(p) = table.get(i) {
                if p.state == State::Exited {
                    to_teardown[count] = i;
                    count += 1;
                }
            }
        }
    }

    for i in 0..count {
        teardown(to_teardown[i], frame_allocator);
    }
}

fn deallocate_user_tables(pml4_frame: PhysFrame, frame_allocator: &mut BootInfoFrameAllocator) {
    use x86_64::structures::paging::page_table::FrameError;

    let offset = crate::memory::get_phys_mem_offset();
    let pml4_virt = offset + pml4_frame.start_address().as_u64();
    let pml4 = unsafe { &*(pml4_virt.as_ptr() as *const PageTable) };

    for pml4_idx in 0..256 {
        let pml4_entry = &pml4[pml4_idx];
        if !pml4_entry.flags().contains(PageTableFlags::PRESENT) {
            continue;
        }
        if !pml4_entry.flags().contains(PageTableFlags::USER_ACCESSIBLE) {
            continue;
        }
        let pdpt_frame = match pml4_entry.frame() {
            Ok(f) => f,
            Err(FrameError::HugeFrame) => panic!("huge page in PML4 (unsupported)"),
            Err(FrameError::FrameNotPresent) => continue,
        };

        let pdpt_virt = offset + pdpt_frame.start_address().as_u64();
        let pdpt = unsafe { &*(pdpt_virt.as_ptr() as *const PageTable) };

        for pdpt_idx in 0..512 {
            let pdpt_entry = &pdpt[pdpt_idx];
            if !pdpt_entry.flags().contains(PageTableFlags::PRESENT) {
                continue;
            }
            if !pdpt_entry.flags().contains(PageTableFlags::USER_ACCESSIBLE) {
                continue;
            }
            let pd_frame = match pdpt_entry.frame() {
                Ok(f) => f,
                Err(FrameError::HugeFrame) => panic!("huge page in PDPT (unsupported)"),
                Err(FrameError::FrameNotPresent) => continue,
            };

            let pd_virt = offset + pd_frame.start_address().as_u64();
            let pd = unsafe { &*(pd_virt.as_ptr() as *const PageTable) };

            for pd_idx in 0..512 {
                let pd_entry = &pd[pd_idx];
                if !pd_entry.flags().contains(PageTableFlags::PRESENT) {
                    continue;
                }
                if !pd_entry.flags().contains(PageTableFlags::USER_ACCESSIBLE) {
                    continue;
                }
                if pd_entry.flags().contains(PageTableFlags::HUGE_PAGE) {
                    if let Ok(big_frame) = pd_entry.frame() {
                        frame_allocator.deallocate_frame(big_frame);
                    }
                    continue;
                }
                let pt_frame = match pd_entry.frame() {
                    Ok(f) => f,
                    Err(FrameError::FrameNotPresent) => continue,
                    Err(FrameError::HugeFrame) => unreachable!(),
                };

                let pt_virt = offset + pt_frame.start_address().as_u64();
                let pt = unsafe { &*(pt_virt.as_ptr() as *const PageTable) };

                for pt_idx in 0..512 {
                    let pt_entry = &pt[pt_idx];
                    if !pt_entry.flags().contains(PageTableFlags::PRESENT) {
                        continue;
                    }
                    if !pt_entry.flags().contains(PageTableFlags::USER_ACCESSIBLE) {
                        continue;
                    }
                    if let Ok(leaf_frame) = pt_entry.frame() {
                        frame_allocator.deallocate_frame(leaf_frame);
                    }
                }

                frame_allocator.deallocate_frame(pt_frame);
            }

            frame_allocator.deallocate_frame(pd_frame);
        }

        frame_allocator.deallocate_frame(pdpt_frame);
    }

    frame_allocator.deallocate_frame(pml4_frame);
}

pub fn validate_user_ptr(vaddr: VirtAddr) -> Option<PhysAddr> {
    let current = CURRENT_PROCESS.load(Ordering::SeqCst);
    if current == 0 {
        return None;
    }

    let cr3 = {
        let table = PROCESSES.lock();
        table.get(current)?.cr3
    };

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
            Err(x86_64::structures::paging::page_table::FrameError::FrameNotPresent) => {
                return None;
            }
            Err(x86_64::structures::paging::page_table::FrameError::HugeFrame) => return None,
        };
        if !entry
            .flags()
            .contains(x86_64::structures::paging::PageTableFlags::USER_ACCESSIBLE)
        {
            return None;
        }
    }

    Some(frame.start_address() + u64::from(vaddr.page_offset()))
}

pub fn validate_user_range(vaddr: VirtAddr, len: usize) -> Option<()> {
    if len == 0 {
        return Some(());
    }
    validate_user_ptr(vaddr)?;
    let end = VirtAddr::new(vaddr.as_u64() + (len - 1) as u64);
    if end.p1_index() == vaddr.p1_index() {
        return Some(());
    }
    validate_user_ptr(end)?;
    Some(())
}
