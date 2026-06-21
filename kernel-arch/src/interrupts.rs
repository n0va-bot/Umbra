use crate::gdt;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use crossbeam_queue::ArrayQueue;
use lazy_static::lazy_static;
use pic8259::ChainedPics;
use spin;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame};

pub const PIC_1_OFFSET: u8 = 32;
pub const PIC_2_OFFSET: u8 = PIC_1_OFFSET + 8;

pub static PICS: spin::Mutex<ChainedPics> =
    spin::Mutex::new(unsafe { ChainedPics::new(PIC_1_OFFSET, PIC_2_OFFSET) });

lazy_static! {
    static ref IDT: InterruptDescriptorTable = {
        let mut idt = InterruptDescriptorTable::new();
        idt.breakpoint.set_handler_fn(breakpoint_handler);
        unsafe {
            idt.double_fault
                .set_handler_fn(double_fault_handler)
                .set_stack_index(gdt::DOUBLE_FAULT_IST_INDEX);
        }
        idt[InterruptIndex::Timer.as_usize()].set_handler_fn(timer_interrupt_handler);
        idt[InterruptIndex::Keyboard.as_usize()].set_handler_fn(keyboard_interrupt_handler);
        idt.page_fault.set_handler_fn(page_fault_handler);
        idt.general_protection_fault
            .set_handler_fn(general_protection_fault_handler);
        idt
    };
}

pub fn init_idt() {
    IDT.load();
}

extern "x86-interrupt" fn breakpoint_handler(stack_frame: InterruptStackFrame) {
    crate::serial_println!("EXCEPTION: BREAKPOINT\n{:#?}", stack_frame);
}

pub static RESCHEDULE_NEEDED: AtomicBool = AtomicBool::new(false);
pub static TICKS: AtomicUsize = AtomicUsize::new(0);

pub static IRQ_ENDPOINTS: spin::Mutex<[Option<usize>; 16]> = spin::Mutex::new([None; 16]);

lazy_static! {
    pub static ref PENDING_IRQS: ArrayQueue<u8> = ArrayQueue::new(256);
}

const TIME_QUANTUM: u64 = 100;

extern "x86-interrupt" fn timer_interrupt_handler(_stack_frame: InterruptStackFrame) {
    let ticks = TICKS.fetch_add(1, core::sync::atomic::Ordering::Relaxed) as u64;

    if ticks > 0 && ticks % TIME_QUANTUM == 0 {
        RESCHEDULE_NEEDED.store(true, core::sync::atomic::Ordering::Release);
    }

    unsafe {
        PICS.lock()
            .notify_end_of_interrupt(InterruptIndex::Timer.as_u8());
    }
}

use x86_64::structures::idt::PageFaultErrorCode;

extern "x86-interrupt" fn page_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: PageFaultErrorCode,
) {
    use x86_64::registers::control::Cr2;

    crate::serial_println!("EXCEPTION: PAGE FAULT");
    crate::serial_println!("Accessed Address: {:?}", Cr2::read());
    crate::serial_println!("Error Code: {:?}", error_code);
    crate::serial_println!("{:#?}", stack_frame);

    // Print the instruction bytes
    if stack_frame.instruction_pointer.as_u64() >= 0x400000 {
        unsafe {
            let ptr = stack_frame.instruction_pointer.as_ptr::<u8>();
            crate::serial_println!(
                "Instruction bytes: {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x}",
                *ptr.offset(0),
                *ptr.offset(1),
                *ptr.offset(2),
                *ptr.offset(3),
                *ptr.offset(4),
                *ptr.offset(5),
                *ptr.offset(6),
                *ptr.offset(7)
            );
        }
    }

    crate::hlt_loop();
}

extern "x86-interrupt" fn general_protection_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) {
    crate::serial_println!("EXCEPTION: GENERAL PROTECTION FAULT");
    crate::serial_println!("Error Code: {:#X}", error_code);
    crate::serial_println!("{:#?}", stack_frame);
    crate::hlt_loop();
}

extern "x86-interrupt" fn keyboard_interrupt_handler(_stack_frame: InterruptStackFrame) {
    let _ = PENDING_IRQS.push(1);
    RESCHEDULE_NEEDED.store(true, Ordering::Release);

    unsafe {
        PICS.lock()
            .notify_end_of_interrupt(InterruptIndex::Keyboard.as_u8());
    }
}

extern "x86-interrupt" fn double_fault_handler(
    stack_frame: InterruptStackFrame,
    _error_code: u64,
) -> ! {
    panic!("EXCEPTION: DOUBLE FAULT\n{:#?}", stack_frame);
}

#[derive(Debug, Clone, Copy)]
#[repr(u8)]

pub enum InterruptIndex {
    Timer = PIC_1_OFFSET,
    Keyboard,
}

impl InterruptIndex {
    fn as_u8(self) -> u8 {
        self as u8
    }

    fn as_usize(self) -> usize {
        usize::from(self.as_u8())
    }
}
