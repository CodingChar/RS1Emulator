# RS1Emu — Roadmap

## Estado actual

Un emulador de PlayStation 1 en Rust que corre en el navegador (WebAssembly) y en el
escritorio (el mismo núcleo, `ps1-core`). Con una BIOS SCPH1001 y un disco de
Resident Evil (USA) arranca la BIOS, lee el disco, muestra la intro en FMV con su audio,
llega al título y a la partida en 3D, con teclado o mando (digital y DualShock), tarjetas
de memoria persistentes y estados guardados completos.

| Parte | Estado |
|---|---|
| CPU R3000A, excepciones, COP0, GTE | completos; intérprete con camino rápido |
| Bus, DMA, timers, interrupciones | completos; dispositivos puestos al día de forma perezosa |
| GPU | rasterizador por software exacto (triángulos, rectángulos, líneas, texturas, transparencias) |
| CD-ROM | BIN/CUE/ISO, comandos, CD-DA con informes, XA 4 y 8 bits con filtro, cambio de disco |
| SPU | 24 voces, ADSR, ruido, FM, reverb, interpolación gaussiana, mezcla de CD/XA |
| MDEC | decodificación de FMV según la documentación |
| Entrada | pad digital, DualShock (analógico, configuración, vibración), 2 puertos, tarjetas de memoria |
| Web | emulador en un worker, disco leído bajo demanda, audio por AudioWorklet, ajustes persistentes |
| Pruebas | 439 tests; benchmark con checksum del estado; ida y vuelta de estados con un juego real |

**Velocidad** (portátil Core i3-8130U; las medidas oscilan ±10 % con la temperatura, por eso
se comparan siempre ejecuciones alternadas): nativo 60–100 M instr/s (2–3× el tiempo real);
WebAssembly en Node 35–50 M (1,0–1,5×); en el navegador, con el emulador en un worker, 100 %
del tiempo real y hasta ~1,9× a velocidad máxima.

Marca `[x]` al completar cada ítem. Los ítems `[ ]` son trabajo pendiente real; los que se
descartaron con una razón técnica están en la sección «Decisiones» del final.

---

## Fase 0 — Base de pruebas

- [x] Tests unitarios en `ps1-core` (`cargo test -p ps1-core --target <host>`): CPU, bus, dispositivos, rasterizador, save states.
- [x] README con cómo compilar y probar en host (`.cargo/config.toml` fuerza `wasm32-unknown-unknown`).
- [x] Harness `examples/boot.rs` (BIOS, disco, capturas, volcados) y test de la imagen de disco local en `rom_test/` (se omite si no existe; la carpeta está en `.gitignore`).
- [x] `examples/bench.rs`: velocidad, checksum de todo el estado, bucles sintéticos, entrada simulada, `--roundtrip` de estados y, con `--features profile`, el reparto del tiempo por área.
- [x] CI (`.github/workflows/ci.yml`): tests nativos y build de WebAssembly.
- [ ] ROMs de prueba de CPU de dominio público (`psxtest_cpu`, suite de `amidog`): hacen falta los binarios, que no están en el repositorio.

## Fase 1 — CPU (R3000A) · HECHA salvo temporización

Archivos: `cpu/interpreter.rs` (referencia, exacto), `cpu/interpreter/fast.rs` (camino rápido).

- [x] Delay slots y de carga, overflow, `lwl/lwr/swl/swr`, `bltzal/bgezal`, todo el repertorio, excepciones y COP0, vector de reset y de excepción.
- [x] Los accesos a direcciones sin dispositivo lanzan excepciones de bus (IBE/DBE) en lugar de detener la emulación.
- [x] Camino rápido: ALU, saltos, cargas/almacenamientos a RAM, scratchpad y BIOS, mult/div, movimientos y comandos del GTE en un bucle con el estado en variables locales; cae al camino exacto ante cualquier otra cosa. Los checksums del benchmark son idénticos con y sin él.
- [ ] Contar ciclos por instrucción y temporizar `mult`/`div` y los accesos a memoria (hoy cada instrucción es un ciclo).

## Fase 2 — Bus y mapa de memoria · HECHA

- [x] Segmentos, espejos de RAM, accesos de 8/16/32 bits, scratchpad, BIOS de solo lectura, cache control.
- [x] Registros de I/O de todos los dispositivos; accesos sin dispositivo registrados en `Bus::unhandled_io`.
- [x] El bus posee la memoria y los dispositivos; RAM y BIOS tienen camino rápido.

## Fase 3 — Temporización, interrupciones y timers · HECHA salvo PAL/entrelazado fino

- [x] Interrupciones, timers 0/1/2 con sus fuentes de reloj, VBlank y hblank, bit par/impar de GPUSTAT.
- [x] Los dispositivos no avanzan en cada ciclo: el bus lleva la cuenta y los pone al día cuando llega su próximo evento (plazo calculado con cotas inferiores) o cuando la CPU toca un registro que depende del tiempo. El resultado es idéntico (comprobado con el checksum) y 2–3× más rápido.
- [ ] Sincronización de los timers 0 y 1 con hblank/vblank (solo el timer 2 detiene su contador) y modo entrelazado fino.

## Fase 4 — Arranque de BIOS · HECHA

- [x] BIOS desde el host o el navegador (512 KB, validada), POST, logo SCE y PlayStation, arranque del juego.
- [x] Ejecutables PS-X EXE (`System::load_exe`): la BIOS arranca y el programa toma el relevo al llegar al shell.
- [ ] Checksum de la BIOS (identificar el modelo).
- [ ] Traza de ejecución detrás de un feature flag.

## Fase 5 — GPU · HECHA salvo temporización

Archivos: `gpu/mod.rs`, `gpu/sw.rs`.

- [x] VRAM de 1024×512, GP0/GP1, GPUSTAT, transferencias, resoluciones y conversión a RGBA reutilizando el búfer.
- [x] Rasterizador: funciones de borde incrementales, tramos por fila, división exacta por recíproco y rellenos por filas. Se comprueba contra la implementación sencilla anterior con pruebas diferenciales aleatorias (triángulos, rectángulos, rellenos, copias).
- [x] El analizador de GP0 no reserva memoria por comando.
- [ ] Temporización de la GPU (flags de ocupado, coste de dibujo).
- [ ] Rellenos, líneas y polígonos exactos al píxel frente a una suite de referencia.

## Fase 6 — DMA · HECHA salvo temporización

- [x] Canales OTC, GPU, CD-ROM, SPU, MDEC in/out; bloque, slice y lista enlazada; las transferencias son instantáneas.
- [ ] Canal PIO, prioridades, troceado y duración real de las transferencias.

## Fase 7 — CD-ROM y formatos de disco

- [x] BIN/CUE (uno o varios archivos), ISO, sectores de 2048/2336/2352 bytes, lectura bajo demanda (archivo del host o función de JavaScript con `FileReaderSync`).
- [x] Controlador completo con retardos, velocidad simple/doble, GetID por región.
- [x] CD-DA (Play por pista), informes de posición (Setmode bit 2), fin de datos al acabar el audio, matriz de volumen.
- [x] XA-ADPCM de 4 y 8 bits, mono/estéreo, filtro de archivo/canal, remuestreo a 44,1 kHz.
- [x] Cambio de disco con la consola en marcha (el estado informa de la tapa abierta hasta el siguiente Getstat).
- [ ] `PREGAP`/`POSTGAP` de los CUE, subcanal Q fiel, Forward/Backward, tiempos de búsqueda según la distancia.
- [ ] Otros formatos: PBP, CCD, CHD, LibCrypt (SBI).

## Fase 8 — GTE · HECHA

- [x] Registros, todas las instrucciones, acumuladores de 44 bits, flags, tabla UNR generada con la fórmula documentada.
- [x] Probado indirectamente con Resident Evil (geometría 3D correcta).
- [ ] Validación con una suite de pruebas del GTE de dominio público y retardos en ciclos de cada comando.

## Fase 9 — Entrada · HECHA salvo multitap

- [x] SIO0, pad digital, DualShock completo (modo analógico, configuración 0x43–0x4D, palancas, vibración con su mapeo), dos puertos, tarjetas de memoria.
- [ ] Multitap y SIO1.

## Fase 10 — Audio (SPU) · HECHA salvo detalles

Archivos: `spu/` (mezclador, voces, reverb, kernel gaussiano), `xa.rs`.

- [x] ADPCM, ADSR, volúmenes con barrido, ruido, modulación de tono, reverb, IRQ, interpolación gaussiana con la tabla de la documentación (validada por su invariante: los cuatro pesos de cada fase suman lo mismo).
- [ ] Filtros FIR de entrada/salida del reverb (sin documentación verificable), buffers de captura y ancho de banda real de las transferencias.
- [ ] Comprobar el audio de oídas en un juego (hoy solo se ha analizado su espectro).

## Fase 11 — MDEC · HECHA salvo temporización

- [x] Decodificación según la documentación: tabla de cuantización por posición de escaneo, caso de escala 0, IDCT con la tabla subida por el juego (13 bits, redondeo documentado), YUV→RGB, salida a 4/8/15/24 bits.
- [x] Canales DMA 0/1 con espera de datos.
- [ ] Decodificación temporizada y estado de bloque/FIFO exactos.

## Fase 12 — Frontend web · HECHA

Archivos: `web/` (`app.js` interfaz, `worker.js` emulador, `audio*.js`, `input.js`, `storage.js`, `settings.js`).

- [x] El emulador corre en un worker; dibuja en un `OffscreenCanvas` leyendo el cuadro de la memoria de WebAssembly sin copiarlo; el audio llega directo al hilo de audio.
- [x] Disco leído bajo demanda con `FileReaderSync` (649 MB sin cargarlos en memoria), BIOS recordada en IndexedDB, programas PS-EXE.
- [x] Teclado reconfigurable, mando estándar, palancas analógicas, botón Analog (F9 / Home), segundo mando en el puerto 2, vibración (Gamepad API).
- [x] Tarjetas de memoria persistentes (exportar, importar, formatear), tres ranuras de estados guardados, cambio de disco en caliente, ajustes (volumen, silencio, velocidad) y teclas guardados.
- [x] Probado en el navegador con el módulo real, la BIOS y el disco de `rom_test/`.
- [ ] Probar la persistencia de las tarjetas con una partida real y el audio con altavoces.
- [ ] Documentación de usuario.

## Rendimiento: lo hecho y lo medido

Cada optimización se validó con el checksum del estado (`bench`), con pruebas diferenciales
(rasterizador) o con la ida y vuelta de un estado (`bench --roundtrip`).

| Cambio | Razón técnica | Efecto |
|---|---|---|
| Dispositivos perezosos con plazos | cada instrucción avanzaba GPU, timers, SPU, CD y SIO | ≈ ×2 |
| Rutas rápidas del bus, errores pequeños | el `Result` con `String` se devolvía por memoria en cada acceso | ≈ ×1,3 |
| Retardo de carga sin `Option` | la copia de `Option<(usize,u32)>` costaba más que la instrucción | ≈ ×1,3 |
| Bucle `run` y camino rápido con estado local | el prólogo de `step` (8 registros) y el estado en memoria | ≈ ×1,5 |
| Traducir la página de código una vez | la tabla de segmentos en la ruta crítica de cada captura | ≈ ×1,6 |
| GTE, LWC2/SWC2 en el camino rápido | 8 M de cargas de coprocesador por 2.500 M de instrucciones | ≈ ×1,1 |
| Rasterizador incremental | 3 funciones de borde y hasta 5 divisiones de 64 bits por píxel | triángulos ×2 |
| GP0 sin asignaciones, cursor de carga | un `Vec` por comando y divisiones por píxel | ≈ 3 % |
| Worker y lectura perezosa | libera el hilo principal y evita 650 MB duplicados | memoria y fluidez |

Globalmente (nativo, mismo trabajo): 21 → 75 M instr/s en el arranque del juego y 11 → 65 M en
el arranque de la BIOS; WebAssembly 6 → 35 M y 10 → 45 M.

## Decisiones (descartado con una razón técnica)

- **SIMD de WebAssembly (`+simd128`)**: medido en ejecuciones alternadas, sin ganancia (la carga es de enteros escalares con saltos); pierde compatibilidad con navegadores antiguos.
- **Caché de bloques o recompilador**: el perfil muestra la CPU en ~80 % pero tras el camino rápido el coste por instrucción ya es de ~12 ns; un recompilador no puede emitir código nativo en WebAssembly y generar módulos al vuelo añade latencia y complejidad. Se reevaluará si hace falta más margen.
- **Backend WebGL2 para la GPU**: la GPU por software ocupa 4–8 % del tiempo tras optimizarla; un renderizador por hardware cambia la semántica de máscaras y transparencias y exigiría una suite de referencia que no hay.
- **`wasm-opt`**: no está instalado y descargarlo no se justifica sin medir antes; el módulo ya se compila con LTO y `opt-level=3`.
- **Dibujo por campos en 480i / `draw_to_display`**: el emulador presenta cuadros completos; ignorar la restricción da la misma imagen final, solo cambia qué líneas se escriben a medio cuadro.
- **Lista de discos recientes**: los navegadores no permiten reabrir archivos locales sin un gesto del usuario, salvo con la File System Access API (solo Chromium).
- **FIR del reverb**: no hay descripción verificable en la documentación consultada; inventar coeficientes empeoraría el sonido.

---

## Hitos de validación

1. **M1** — Los tests de CPU pasan. ✔
2. **M2** — La BIOS ejecuta sin errores durante varios cuadros. ✔
3. **M3** — Se ve la pantalla de arranque de la BIOS. ✔
4. **M4** — Arranca un juego de prueba con imagen y entrada. ✔
5. **M5** — Juego 3D con audio, FMV y partida jugable en el navegador. ✔ (Resident Evil; el audio se comprobó por espectro, no de oídas).
