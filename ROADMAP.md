# RS1Emu — Roadmap

Estado actual: la CPU (R3000A) y el GTE están completos; el bus mapea RAM, BIOS,
scratchpad y todos los registros de I/O; hay temporización de vídeo (VBlank), timers con
contadores reales, DMA, el controlador del CD-ROM con lectura de BIN/CUE, el puerto serie
SIO0 con pad digital y memory cards, y una GPU con rasterizador por software (369 tests
unitarios). Con una BIOS SCPH1001 real y un disco, la BIOS muestra su logo, identifica el
disco, carga el ejecutable del juego y se lo cede; Resident Evil (USA) muestra el logo de
CAPCOM y su pantalla de título (~1.200 M de instrucciones). El MDEC decodifica el FMV de introducción (Resident Evil lo muestra entero antes del título). Falta la SPU (audio) y el
intérprete va a ~15 M instr/s en nativo, la mitad del tiempo real.

Las fases están ordenadas por prioridad: cada una depende de las anteriores.
Marca `[x]` al completar cada ítem.

---

## Fase 0 — Base de pruebas (en paralelo con la Fase 1)

- [x] Tests unitarios en `ps1-core` (`cargo test -p ps1-core --target x86_64-pc-windows-msvc`): CPU, bus y save state.
- [x] Documentar en el README cómo compilar/testear en host, ya que `.cargo/config.toml` fuerza `wasm32-unknown-unknown`.
- [x] Harness de arranque `cargo run -p ps1-core --example boot` y test que valida la imagen de disco local en `rom_test/` (se omite si no existe; la carpeta está en `.gitignore`).
- [ ] Integrar ROMs de test de CPU de dominio público (p. ej. `psxtest_cpu`, suite de `amidog`), sin incluir BIOS ni juegos.
- [ ] CI básico: `cargo check`, `cargo test` y build wasm.

## Fase 1 — CPU (R3000A) · HECHA (salvo temporización)

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
- [x] Instrucciones COP2 y `lwc2`/`swc2` (Fase 8).

## Fase 2 — Bus y mapa de memoria · HECHA (faltan excepciones de bus)

Archivo: `crates/ps1-core/src/bus.rs`

- [x] Traducción de segmentos: KUSEG, KSEG0 (`0x8000_0000`), KSEG1 (`0xA000_0000`); aplicar máscara en vez de comparar con `0x8000_0000`.
- [x] Mapa físico correcto: RAM (2 MB con espejos x4), expansión 1 (bus abierto), scratchpad, I/O + expansión 2, BIOS (solo lectura) y cache control en KSEG2.
- [x] Accesos de 8, 16 y 32 bits reales (`read8`/`read16`/`read32` y `write*`), sin truncar palabras.
- [x] Scratchpad real (1 KB).
- [x] Registros de I/O mapeados: interrupt stat/mask, DMA, timers, GPU (GP0/GP1/GPUSTAT), CD-ROM, SPU, SIO y MDEC. El comportamiento real de cada dispositivo llega en su fase; CD-ROM, SIO y MDEC son por ahora registros que devuelven lo escrito.
- [ ] Registrar en el bus los accesos a direcciones de I/O sin dispositivo (hecho: `Bus::unhandled_io`) y revisarlos al arrancar la BIOS real.
- [x] Registros de configuración de memoria (`0x1F80_1000`–`0x1F80_1060`) y cache control (`0xFFFE_0130`).
- [ ] Alineación y excepciones de bus (`MemoryOutOfRange` hoy aborta la emulación).
- [x] El bus es dueño de la memoria y de los dispositivos; la CPU solo recibe `&mut Bus`.

## Fase 3 — Temporización, interrupciones y timers

- [x] Controlador de interrupciones: `I_STAT`/`I_MASK` y enrutado a COP0 CAUSE bit 10 (`System::step`). Faltan las fuentes (VBlank, timers, DMA, CD-ROM...).
- [x] Timers 0, 1, 2 con fuentes de reloj (sysclk, dotclock, hblank, sysclk/8), modos y IRQ (`timers.rs`). Falta la sincronización con hblank/vblank de los timers 0 y 1 (solo el timer 2 detiene su contador).
- [ ] Planificador por eventos (como `timing_event` de DuckStation) en lugar de avanzar todos los componentes un ciclo por instrucción.
- [x] Constantes de vídeo NTSC: 3413 ciclos de GPU por línea, 263 líneas, 571212 ciclos de CPU por cuadro.
- [ ] Modo PAL (314 líneas), entrelazado y divisor del dot clock según la resolución (GP1 08h).
- [x] VBlank (línea 240) dispara IRQ0; el hblank alimenta al timer 1.
- [x] Bit 31 de GPUSTAT (línea par/impar) implementado en `Gpu::status()`.

## Fase 4 — Arranque de BIOS

Hito: la BIOS llega a la pantalla de arranque / shell.

- [x] Cargar la BIOS desde el host (`examples/boot.rs --bios`); en la web, el selector de archivo sigue pendiente.
- [ ] Validar tamaño/checksum de la BIOS.
- [x] Harness `examples/boot.rs`: ejecuta N instrucciones, imprime los códigos POST, los últimos `pc` y los accesos a I/O sin dispositivo.
- [x] Desensamblador y listado de bucles calientes en el harness (`--disasm`, `--peek`).
- [ ] Traza de ejecución (cpu trace, opcional detrás de feature flag).
- [x] Hito parcial: con `SCPH1001.BIN` la BIOS recorre los POST 0x01–0x07, instala el kernel en RAM, usa DMA/VBlank, inicializa el CD-ROM y reconoce el disco (GetID, lectura de los sectores de licencia).
- [x] El bloqueo del GTE (`ctc2 t0, cop2r29`, tras ~137 M de instrucciones) quedó resuelto en la Fase 8; la BIOS alcanza el POST 0x09 (~172 M de instrucciones).
- [x] El bucle de espera en `JOY_STAT` (SIO0) quedó resuelto en la Fase 9: la BIOS arranca el juego (SLUS_001.70) y le cede el control.
- [x] El supuesto bloqueo en `VSync` era falso: el juego solo necesitaba más tiempo emulado (ver Fase 5); el contador de libgpu avanza al ritmo del VBlank.
- [x] Hito: la BIOS muestra el logo SCE (~160 M de instrucciones) y el de PlayStation con la licencia (~260 M).
- [ ] Soporte para ejecutables PS-EXE para pruebas sin disco.

## Fase 5 — GPU

Archivos: `gpu/mod.rs`, `gpu/sw.rs`, `gpu/backend.rs`

- [x] VRAM de 1024×512 a 16 bpp dentro de la GPU (`gpu/sw.rs::Vram`).
- [x] Registros GP0/GP1 y GPUSTAT; parser de comandos GP0 (comandos, polilíneas, cargas de píxeles).
- [x] Transferencias VRAM: CPU→VRAM, VRAM→CPU, VRAM→VRAM, fill rectangle.
- [x] Rasterizador software: triángulos con Gouraud, texturas (4/8/15 bpp, CLUT, ventana de textura), semi-transparencia, dithering, máscara, rectángulos y líneas.
- [x] Display area y resoluciones (256/320/368/512/640 × 240/480), conversión VRAM→RGBA de 15/24 bpp; el frontend redimensiona el canvas cada cuadro.
- [x] Hito: se ve la imagen del juego (logo de CAPCOM y pantalla de título de Resident Evil con la BIOS real).
- [x] Harness: `--screenshot`, `--vram`, `--shots-every N --shots-prefix P` y `--watch-pc` guardan PNG / cuentan ejecuciones.
- [ ] Temporización de la GPU (flags de ocupado, coste de dibujo) y caché de texturas.
- [ ] Entrelazado 480i con dibujo por campos y restricción `draw_to_display`.
- [ ] Rellenos, líneas y polígonos exactos al píxel frente a una suite de referencia.
- [ ] Implementar `GpuBackend` para WebGL2 (hoy `ps1-web` usa canvas 2D).

## Fase 6 — DMA

Archivo: `dma.rs`

- [x] Registros por canal (MADR, BCR, CHCR), DPCR, DICR e IRQ.
- [x] Modos: bloque, slice y lista enlazada (GPU); las transferencias terminan al instante.
- [x] Canales: OTC, GPU (a/desde RAM y lista enlazada), SPU (memoria de sonido) y CD-ROM (FIFO de datos del sector).
- [x] Canales MDEC in/out (Fase 11).
- [ ] Canal PIO.
- [ ] Prioridad entre canales, troceado (chopping) y duración real de las transferencias (hoy no detienen a la CPU).

## Fase 7 — CD-ROM y formatos de disco

Archivo: `cdrom.rs`

Archivo: `disc.rs` (imágenes)

- [x] Parser de imágenes: BIN crudo (2352), ISO (2048) y hojas CUE con uno o varios archivos (`Disc::from_cue`); `FileSource` lee los sectores bajo demanda, sin cargar 680 MB en memoria.
- [x] Lectura de sectores (2048, 2336 y 2352 bytes), pistas, TOC y tiempo MSF; detección de región por el sector de licencia.
- [x] Controlador del CD-ROM: registros, FIFOs de parámetros/respuesta/datos, interrupciones INT1–INT5 con retardos, comandos Getstat, Setloc, ReadN/ReadS, Pause, Stop, MotorOn, Init, Setmode, Setfilter, Getparam, GetlocL/P, GetTN, GetTD, SeekL/P, Test, GetID, ReadTOC, SetSession, y velocidad simple/doble.
- [x] Registro de comandos (`CdRom::command_log`) y salida en el harness.
- [ ] Pistas de audio: Play/Forward/Backward solo responden, no reproducen; CDDA y XA-ADPCM.
- [ ] Filtro XA real (Setfilter solo guarda los valores), modo de informe (Report) y auto-pausa.
- [ ] `PREGAP`/`POSTGAP` de las hojas CUE y subcanal Q (GetlocP aproxima la posición).
- [ ] Tiempos de búsqueda realistas según la distancia; hoy son retardos fijos.
- [ ] Cargar el disco desde el navegador sin copiar todo el archivo a memoria (lectura por trozos con `File.slice`).
- [ ] Soporte posterior: PBP, CCD, CHD, multi-disco, protección LibCrypt (SBI).

## Fase 8 — GTE (coprocesador 2)

Archivo: `gte.rs`

- [x] Registros de datos y control del COP2 (32 + 32) con sus particularidades de lectura/escritura (registros de 16 bits con signo, FIFO de SXYP, IRGB/ORGB, LZCS/LZCR, bit 31 de FLAG).
- [x] `mfc2`/`cfc2`/`mtc2`/`ctc2`/`lwc2`/`swc2`, todas condicionadas a `SR.CU2`.
- [x] Instrucciones: RTPS/RTPT, NCLIP, MVMVA (con el fallo del color lejano y la matriz basura), AVSZ3/4, OP, GPF/GPL, DPCS, INTPL, SQR, NCS/NCT, NCDS/NCDT, NCCS/NCCT, CC, CDP, DCPL, DPCT, y la tabla UNR del divisor.
- [x] Acumuladores MAC de 44 bits y todos los flags de saturación.
- [ ] Validar contra una suite de pruebas del GTE de dominio público (los tests actuales usan valores calculados a mano; hay detalles de redondeo y de flags, como IR3 en RTPS o el uso de MAC0 en OTZ, que solo una suite o un juego pueden confirmar).
- [ ] Comandos con efectos indefinidos (opcodes sin asignar) y el retardo en ciclos de cada comando.

## Fase 9 — Entrada (SIO / pads / memory cards)

Archivos: `controllers/mod.rs`, `controllers/pad.rs`

- [x] Registros SIO0 (JOY_DATA, STAT, MODE, CTRL, BAUD) con FIFO de recepción, temporización por baudios, ACK e IRQ7.
- [x] Protocolo del pad digital (0x01 0x42) y puerto vacío (la línea queda a 0xFF sin ACK).
- [ ] Pad analógico (DualShock): modos 0x73/0x79, comandos de configuración (0x43, 0x44, 0x45...), palancas y vibración.
- [x] Memory cards: lectura, escritura (con checksum y sector inválido) e ID; formato vacío válido; indicador de escritura (`dirty`) y API `System`/`Ps1` para insertar y extraer la imagen.
- [ ] Persistencia de las memory cards en IndexedDB desde `ps1-web` (el núcleo ya expone `memory_card_data`/`memory_card_dirty`).
- [ ] Guardar en el save state el estado de SIO0 y de las memory cards.
- [ ] Teclado y gamepad API en `ps1-web` mapeados a `set_input`.
- [ ] Multitap (el `ControllerManager` ya reserva 8 puertos).
- [ ] SIO1 (puerto serie/cable link): hoy solo registros que devuelven lo escrito.

## Fase 10 — Audio (SPU)

Archivo: `spu.rs`

- [ ] Registros SPU (voces, volúmenes, ADSR, control, estado) mapeados en el bus; unificar `sound_ram` con `Memory::spu_ram` (hoy están duplicadas).
- [ ] Decodificación ADPCM, mezcla de 24 voces, envolventes, ruido, FM.
- [ ] Reverb.
- [ ] Mezcla de CD-DA/XA.
- [ ] Salida a Web Audio API (AudioWorklet) desde `ps1-wasm`/`ps1-web`.

## Fase 11 — MDEC (vídeos FMV)

- [x] Módulo `mdec.rs`: comandos (decodificar, tablas de cuantización, tabla de escala), registros de datos/estado/control, decodificación RLE, desfase DC/AC con zigzag, IDCT con la tabla subida por el juego, conversión YCbCr→RGB a 24/15 bpp y salida monocroma a 8/4 bpp.
- [x] Canales DMA 0 (entrada) y 1 (salida); una salida lanzada antes de que el decodificador tenga los datos espera y termina al llegar.
- [x] Hito: el vídeo de introducción de Resident Evil se ve entero (capturas con `--shots-every`).
- [ ] Decodificación temporizada (hoy es instantánea) y bits de estado de bloque actual / FIFO exactos.
- [ ] Verificar el redondeo del IDCT y la indexación de la tabla de cuantización contra una referencia (líneas verticales tenues en el vídeo).
- [ ] Incluir el MDEC en el guardado de estados.

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
