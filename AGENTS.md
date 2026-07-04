# Umbra — Agent Guide

## Project

x86_64 hobby OS kernel (microkernel, following Philipp Oppermann's tutorial). **no_std, no_main, nightly Rust.** Cargo Workspace containing the kernel, userspace shell, and builder.

## Toolchain

- **Nightly required.** No `rust-toolchain.toml` — relies on `rustup override`. Active: `nightly-x86_64-unknown-linux-gnu` (directory override).
- **Editions:** 2024.
- **Extra components needed:**
  ```
  rustup component add llvm-tools-preview
  cargo install bootimage
  ```

## Build & Run

| Command | What it does |
| --- | --- |
| `cargo run` | Builds the kernel and launches in QEMU via the custom `builder` package |
| `cargo check` | Works (only type-checks, no linker needed) |

The project uses a custom `builder` package to compile the kernel into a UEFI/BIOS disk image using `bootloader` 0.11, rather than `bootimage`.

## Testing

Tests **run inside QEMU**, not natively. Uses custom test framework with serial output.

- **All tests:** `cargo test` (runs every test in QEMU)
- **Single test:** `cargo test --test <name>` (e.g. `basic_boot`, `heap_allocation`)
- **Test exit codes:** success = 33, failure = 0x11 (via `isa-debug-exit` device at port 0xf4)
- **Test harness:** custom (`#![feature(custom_test_frameworks)]` with `umbra::test_runner`)
- **Tests with `harness = false`:** `should_panic`, `stack_overflow` — define their own `_start`
- **Test output:** serial (not VGA). Test args: `-serial stdio -display none`
- **Timeout:** 300s per test

## Key Architecture

- **`src/lib.rs`** — library root: `init()`, `hlt_loop()`, test framework, QEMU exit
- **`src/main.rs`** — kernel entry (`kernel_main` via `bootloader::entry_point!`)
- **`src/vga_buffer.rs`** — VGA text mode writer + `print!`/`println!` macros
- **`src/serial.rs`** — serial port output (tests use this)
- **`src/interrupts.rs`** — IDT setup: breakpoint, double fault, timer, keyboard, page fault
- **`src/gdt.rs`** — GDT + TSS with IST for double faults
- **`src/memory.rs`** — page tables, `OffsetPageTable`, `BootInfoFrameAllocator`
- **`src/allocator.rs`** — `linked_list_allocator::LockedHeap` at `0x4444_4444_0000`, 100 KiB
- **`src/task/`** — async executor (`Executor` + `SimpleExecutor`), keyboard task

## Constraints

- `panic = "abort"` in all profiles (dev, release, test) — no unwinding
- `#![no_std]` everywhere — no `std` crate available
- `#![feature(abi_x86_interrupt)]` required for interrupt handlers
- `#![feature(custom_test_frameworks)]` required for tests
- VGA buffer at `0xb8000`, serial COM1 at `0x3F8`
- Global descriptor table and IDT must be initialized before interrupts
- Heap initialized late (after paging setup) — some things unavailable before init

## Gotchas

- **Do not use `std`** — everything is `core` + `alloc`
- **Do not add standard Rust tests** — all tests are custom-harness integration tests that boot in QEMU
- **Do not change `edition`** — the project uses 2024 with nightly features
- **Do not add new dependencies without verifying `no_std` compatibility**
- **Never use `println!` in test panic handlers** — tests use `serial_print!`/`serial_println!`
- **Cargo.lock must be committed** (it is checked in)
- **No CI, no Makefile, no pre-commit hooks**
- **To push, use the ./push script**
- **[ ] means task not completed, [X] means task completed, [/] means task in progress**
- **If a write/file creation operation fails because the file already exists, edit the file instead of removing and rewriting it**
- **THERE IS NO SUCH THING AS 'ACCEPTABLE FOR A HOBBY OS'**