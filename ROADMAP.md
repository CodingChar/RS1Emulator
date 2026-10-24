# RS1Emu — Roadmap

Estado actual: la CPU (R3000A) está completa salvo el GTE, con delay slots, load delay,
excepciones y COP0, y el bus traduce segmentos y hace accesos de 8/16/32 bits a
RAM/BIOS/scratchpad (61 tests unitarios). El resto de los componentes son stubs, y el
framebuffer siempre está en negro.

Las fases están ordenadas por prioridad: cada una depende de las anteriores.
Marca `[x]` al completar cada ítem.

---

## Fase 0 — Base de pruebas (en paralelo con la Fase 1)

- [x] Tests unitarios en `ps1-core` (`cargo test -p ps1-core --target x86_64-pc-windows-msvc`): CPU, bus y save state.
- [ ] Documentar en el README cómo compilar/testear en host, ya que `.cargo/config.toml` fuerza `wasm32-unknown-unknown`.
- [ ] Integrar ROMs de test de CPU de dominio público (p. ej. `psxtest_cpu`, suite de `amidog`), sin incluir BIOS ni juegos.
- [ ] CI básico: `cargo check`, `cargo test` y build wasm.

## Fase 1 — CPU (R3000A) · HECHA (salvo GTE y temporización)

Archivo: `crates/ps1-core/src/cpu/interpreter.rs`

**Correcciones de lo existente**
- [x] Delay slots reales: `j`, `jal`, `jr`, `jalr`, `beq`, `bne` deben ejecutar la instrucción siguiente antes de saltar. Incluir el slot de carga (load delay).
- [x] `addi`/`addiu`: extender el signo del inmediato; `addi`, `add` y `sub` deben detectar overflow y lanzar excepción.
- [x] `lb`/`lbu`: `lb` con extensión de signo, `lbu` con extensión a cero.
- [x] Corregir el opcode de `lwr` (0x26) e implementar `lwl`/`lwr`/`swl`/`swr`.
- [x] Rama condicional: calcular el destino con el inmediato extendido en signo.

**Instrucciones faltantes**
- [x] Saltos y comparaciones: `bgtz`, `blez`, `bltz`, `bgez`, `bltzal`, `bgezal`, `slti`, `sltiu`.
- [x] Lógicas con inmediato: `ori`, `xori`.
- [x] Desplazamientos variables: `sllv`, `srlv`, `srav`.
- [x] Memoria: `lh`, `lhu`, `sh`.
- [x] `syscall`, `break`.

**Excepciones y COP0**
- [x] Registros COP0 (SR, CAUSE, EPC, BadVAddr), `mfc0`, `mtc0`, `rfe`.
- [x] Excepciones: overflow, dirección desalineada, instrucción reservada, syscall, break, interrupciones.
- [x] Vector de excepción (0xBFC00180 o 0x80000080 según BEV).
- [x] Arranque en el vector de reset `0xBFC00000`.

**Estructura**
- [x] `UnimplementedOpcode` queda solo para COP2/GTE con `SR.CU2` activo; los opcodes reservados ahora lanzan la excepción RI. Los fallos del bus (`MemoryOutOfRange`) siguen siendo errores fatales del emulador.
- [ ] Contar ciclos por instrucción (base para la temporización de la Fase 3).
- [ ] Temporización de `mult`/`div` y de los accesos a memoria; interlocks de HI/LO.
- [ ] Instrucciones COP2 y `lwc2`/`swc2` (dependen de la Fase 8).

## Fase 2 — Bus y mapa de memoria · prioridad máxima (junto con la Fase 1)

Archivo: `crates/ps1-core/src/bus.rs`

- [x] Traducción de segmentos: KUSEG, KSEG0 (`0x8000_0000`), KSEG1 (`0xA000_0000`); aplicar máscara en vez de comparar con `0x8000_0000`.
- [ ] Mapa físico correcto: ya cubre RAM (2 MB con espejos x4) y BIOS (solo lectura); faltan I/O (`0x1F80_1000`+) y regiones de expansión.
- [x] Accesos de 8, 16 y 32 bits reales (`read8`/`read16`/`read32` y `write*`), sin truncar palabras.
- [ ] Scratchpad real (hoy devuelve 0).
- [ ] Registros de I/O: interrupt stat/mask, DMA, timers, GPU (GP0/GP1), CD-ROM, SPU, pads/memory cards (SIO), MDEC.
- [ ] Registros de configuración de memoria (`0x1F80_1000`–`0x1F80_1060`) y cache control (`0xFFFE_0130`).
- [ ] Alineación y excepciones de bus (`MemoryOutOfRange` hoy aborta la emulación).
- [ ] Reemplazar los `memory: &mut Memory` repartidos por un diseño donde el bus sea dueño de los dispositivos (evita pasar `Memory` a todos los `step`).

## Fase 3 — Temporización, interrupciones y timers

- [ ] Controlador de interrupciones: `stat`/`mask`, enrutado a COP0 CAUSE bit 10 (`interrupt.rs`, hoy `step` no hace nada).
- [ ] Timers 0, 1, 2 con fuentes de reloj (sysclk, dotclock, hblank, sysclk/8), modos y IRQ (`timers.rs`).
- [ ] Planificador por eventos (como `timing_event` de DuckStation) en lugar de avanzar todos los componentes un ciclo por instrucción.
- [ ] Constantes de vídeo reales: ciclos por línea y por cuadro NTSC/PAL (hoy `cycles_per_frame = 3413`, que es un ciclo de línea, no de cuadro).
- [ ] VBlank / HBlank que disparen IRQ.

## Fase 4 — Arranque de BIOS

Hito: la BIOS llega a la pantalla de arranque / shell.

- [ ] Cargar BIOS vía UI web (selector de archivo) y vía host.
- [ ] Validar tamaño/checksum de la BIOS.
- [ ] Traza de ejecución y desensamblador para depurar (cpu trace, opcional detrás de feature flag).
- [ ] Soporte para ejecutables PS-EXE para pruebas sin disco.

## Fase 5 — GPU

Archivos: `gpu/mod.rs`, `gpu/sw.rs`, `gpu/backend.rs`

- [ ] VRAM de 1024×512 a 16 bpp (hoy el framebuffer es 320×240 RGBA y `vram` es un `Vec<u8>` sin uso).
- [ ] Registros GP0/GP1 y GPUSTAT; parser de comandos GP0.
- [ ] Transferencias VRAM: CPU→VRAM, VRAM→CPU, VRAM→VRAM, fill rectangle.
- [ ] Rasterizador software: triángulos con Gouraud, texturas (4/8/15 bpp, CLUT), semi-transparencia, dithering, rectángulos y líneas.
- [ ] Display area y resoluciones (256/320/368/512/640 × 240/480), conversión VRAM→RGBA para presentación.
- [ ] Implementar `GpuBackend` para WebGL2 (hoy `ps1-web` usa canvas 2D).

## Fase 6 — DMA

Archivo: `dma.rs`

- [ ] Registros por canal (MADR, BCR, CHCR), DPCR, DICR e IRQ.
- [ ] Modos: bloque, slice y lista enlazada (GPU), orden de prioridad.
- [ ] Canales: MDEC in/out, GPU, CD-ROM, SPU, PIO, OTC.

## Fase 7 — CD-ROM y formatos de disco

Archivo: `cdrom.rs`

- [ ] Parser de imágenes: ISO, BIN+CUE (hoy `insert_image` no parsea nada y `total_sectors = 0`).
- [ ] Lectura de sectores (2048 y 2352 bytes), TOC y tiempo MSF.
- [ ] Controlador del CD-ROM: registros, FIFOs de parámetros/respuesta, comandos (Getstat, Setloc, ReadN, Pause, Init, Setmode, GetID, Seek, ...), IRQs con retardos.
- [ ] Soporte posterior: PBP, CCD, multi-disco, XA-ADPCM.

## Fase 8 — GTE (coprocesador 2)

Archivo: `gte.rs`

- [ ] Registros de datos y control del COP2 (hoy `regs: [u32; 32]` solo cubre uno de los dos bancos).
- [ ] `mfc2`/`cfc2`/`mtc2`/`ctc2`/`lwc2`/`swc2`.
- [ ] Instrucciones: RTPS/RTPT, NCLIP, MVMVA, AVSZ3/4, OP, GPF/GPL, DPCS, INTPL, SQR, NCS/NCT, NCDS/NCDT, NCCS/NCCT, CC, CDP, DCPL, DPCT, y tablas UNR.
- [ ] Flags de saturación y tests contra resultados conocidos.

## Fase 9 — Entrada (SIO / pads / memory cards)

Archivos: `controllers/mod.rs`, `controllers/pad.rs`

- [ ] Registros SIO0 (JOY_DATA, STAT, MODE, CTRL, BAUD) e IRQ.
- [ ] Protocolo del pad digital (0x01 0x42), luego analógico (DualShock) y vibración.
- [ ] Memory cards: protocolo y persistencia (IndexedDB desde `ps1-wasm`).
- [ ] Teclado y gamepad API en `ps1-web` mapeados a `set_input`.
- [ ] Multitap (el `ControllerManager` ya reserva 8 puertos).

## Fase 10 — Audio (SPU)

Archivo: `spu.rs`

- [ ] Registros SPU (voces, volúmenes, ADSR, control, estado) mapeados en el bus; unificar `sound_ram` con `Memory::spu_ram` (hoy están duplicadas).
- [ ] Decodificación ADPCM, mezcla de 24 voces, envolventes, ruido, FM.
- [ ] Reverb.
- [ ] Mezcla de CD-DA/XA.
- [ ] Salida a Web Audio API (AudioWorklet) desde `ps1-wasm`/`ps1-web`.

## Fase 11 — MDEC (vídeos FMV)

- [ ] Módulo nuevo `mdec.rs`: decodificación de macrobloques, tablas IDCT, canal DMA in/out.

## Fase 12 — Frontend web y herramientas

- [ ] UI en `ps1-web`: cargar BIOS y disco desde el navegador (hoy el `main.rs` crea `Ps1` y corre cuadros sin BIOS ni disco).
- [ ] Controles: pausa, reset, velocidad, pantalla completa.
- [ ] Guardado de estados completo y versionado (hoy `SaveState` no cubre GPU, timers/IRQ internos ni CD-ROM, y falla en silencio si cambia la versión); persistir en IndexedDB.
- [ ] Ajustes persistentes (`settings.rs` hoy solo guarda en memoria).
- [ ] Rendimiento: perfilar el intérprete, evaluar cachés de bloques/threaded code; Web Workers para emulación fuera del hilo principal.
- [ ] Documentación de usuario y de arquitectura.

---

## Hitos de validación sugeridos

1. **M1** — Los tests de CPU de la Fase 0 pasan.
2. **M2** — La BIOS ejecuta sin errores durante varios cuadros (Fases 1–4).
3. **M3** — Se ve la pantalla de arranque de la BIOS (Fase 5).
4. **M4** — Arranca un juego de prueba con imagen y entrada (Fases 6, 7, 9).
5. **M5** — Juegos 3D con audio jugables (Fases 8, 10).
