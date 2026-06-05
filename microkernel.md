# What "more microkernel-y" would actually look like

A microkernel does **almost nothing** in supervisor mode. Privilege is reserved
for: scheduling, IPC, address-space management, interrupt dispatch, capability
checking. Everything else — framebuffer, keyboard, RTC, PCI scan, ACPI,
filesystem, net stack — runs in userspace and talks to the kernel only via IPC.

Concrete changes, roughly in the order I'd do them:

1. **Multi-process core** — process table, per-process page tables, kernel
   stack per process, a real scheduler (round-robin is fine to start), `yield`
   / preemptive switch on the timer interrupt. Right now `iretq` into the
   shell is a one-way trip with a TODO.

2. **Collapse the 7 syscalls into one IPC primitive.** Replace
   `syscall.rs`'s `match syscall_nr` with `ipc_send(endpoint_cap, msg)` +
   `ipc_recv(endpoint_cap, msg)` (or `ipc_call = send+recv+reply` in one
   shot). The "syscall number" stops being a thing; a process can only do
   what its capabilities allow.

3. **Move drivers out of kernel.**
   - `framebuffer` → userspace `fb-server` process. Kernel only exposes
     `mmap_framebuffer()` (map the physical FB region into the caller's page
     table, with the right `USER_ACCESSIBLE` flags). The server then renders
     to it directly.
   - `cmos` (RTC) → userspace `rtc` process, given a port-I/O capability for
     ports `0x70`/`0x71`.
   - `pci` → userspace `pci-arbiter` walks the bus, owns the `0xCF8`/`0xCFC`
     port pair, hands out per-device capabilities to whoever asks.
   - `acpi::power_off` → userspace power service, given a port-I/O cap for
     PM1a.
   - `serial` → can stay in-kernel for now (boot logs), or become a
     userspace `tty` server. Either is defensible.

4. **Capabilities.** Each process has a small CSpace (capability table). To
   do port I/O, you need a `Port` cap; to map a physical region, a `Frame`
   cap; to talk to another process, an `Endpoint` cap. The shell starts with
   caps to the fb-server, the rtc, the pci-arbiter, the power service, the
   keyboard driver — and nothing else. This is the seL4/L4 model.

5. **Split the kernel into workspace crates.** The current single `umbra`
   crate is the second-biggest monolith-ish smell after the syscall table. A
   reasonable split:
   - `kernel-arch` (or `kernel/arch/x86_64`) — GDT, IDT, syscall entry,
     context switch, port I/O traps.
   - `kernel-core` — process table, scheduler, IPC, capability table,
     page-table management.
   - `kernel-hal` — the minimal trait the arch and core depend on (clock,
     frame allocator trait, etc.).
   - `kernel` — thin binary that wires it all up and boots.
   - `userspace/libumbra` — `cap`, `ipc`, `process::spawn` helpers shared
     by all userspace binaries.
   - `userspace/{shell,fb-server,rtc,pci-arbiter,ps2-kbd,power}` — one
     binary each, or a `userspace/services` crate with multiple `[[bin]]`
     entries.

6. **Clean the boot path.** `lib.rs::init` currently does only the CPU
   init. Rename / split: `arch::init()` (GDT/IDT/PIC), `mem::init(boot_info)`,
   `ipc::init()`, `process::init()`. `kernel_main` becomes a top-to-bottom
   `init()` chain with no inline `mapper.map_to` for the user stack (that
   becomes a `process::spawn` helper).
