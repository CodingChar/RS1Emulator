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
| Web | emulador en un worker, disco leído bajo demanda, audio por AudioWorklet, ajustes persistentes; menú de ventana, menú rápido a pantalla completa, mandos USB/Bluetooth configurables |
| Imagen y audio | aspectos 4:3, 16:9, ampliar sin deformar, escala entera y rellenar, recorte del borde; escalado interno 1×/2×/3×, PGXP, texturas con perspectiva, filtro bilineal, color verdadero, panel de rendimiento; audio con reserva estable y control dinámico de la velocidad |
| Temporización | latencia de mult/div y del GTE, GPU ocupada mientras dibuja, DMA con duración (y troceado), búsqueda del CD según la distancia, timers 0/1 sincronizados con el blanking |
| Pruebas | 507 tests (y 3 más, ignorados, que arrancan la BIOS y juegos reales); benchmark con checksum del estado; ida y vuelta de estados con un juego real |

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
- [x] `tests/boot_games.rs`: Crash Bandicoot llega al menú del título y Resident Evil lee la tarjeta de memoria desde LOAD GAME (ignorados por defecto, ~1 min cada uno: `cargo test --release ... --test boot_games -- --ignored`; se omiten sin BIOS o disco).
- [x] `tests/memory_card_bios.rs`: el gestor de tarjetas de la BIOS, manejado con pulsaciones, copia una partida de una tarjeta a otra con sus servicios de lectura y escritura (ignorado, ~30 s; se omite sin BIOS).
- [x] CI (`.github/workflows/ci.yml`): tests nativos y build de WebAssembly.
- [ ] ROMs de prueba de CPU de dominio público (`psxtest_cpu`, suite de `amidog`): hacen falta los binarios, que no están en el repositorio.

## Fase 1 — CPU (R3000A) · HECHA salvo temporización

Archivos: `cpu/interpreter.rs` (referencia, exacto), `cpu/interpreter/fast.rs` (camino rápido).

- [x] Delay slots y de carga, overflow, `lwl/lwr/swl/swr`, `bltzal/bgezal`, todo el repertorio, excepciones y COP0, vector de reset y de excepción.
- [x] Los accesos a direcciones sin dispositivo lanzan excepciones de bus (IBE/DBE) en lugar de detener la emulación.
- [x] Camino rápido: ALU, saltos, cargas/almacenamientos a RAM, scratchpad y BIOS, mult/div, movimientos y comandos del GTE en un bucle con el estado en variables locales; cae al camino exacto ante cualquier otra cosa. Los checksums del benchmark son idénticos con y sin él.
- [x] Temporizar `mult`/`div` (6/9/13 y 36 ciclos): el resultado se lee con espera solo si se pide antes de tiempo, igual en el camino rápido y en el exacto (pruebas de equivalencia). Un lote del camino rápido corta la espera al final para no saltarse ningún evento de dispositivo.
- [x] Cobrar la latencia de las lecturas de RAM (`RAM_READ_CYCLES`, 5 ciclos más por lectura, igual en el camino rápido y en el exacto, con pruebas de equivalencia). La referencia que la valida: Resident Evil espera la tarjeta con un bucle de iteraciones y daba `ACCESS ERROR` con 1 ciclo por lectura (ver `docs/errores-conocidos.md`).
- [ ] Resto de la temporización de memoria: escrituras con su búfer, lecturas de BIOS y de E/S, y búsqueda de instrucciones con la caché de 4 KB. Sin una referencia que lo valide cambiaría la velocidad de los juegos a ciegas; las lecturas de RAM, que sí tienen una, ya están.

## Fase 2 — Bus y mapa de memoria · HECHA

- [x] Segmentos, espejos de RAM, accesos de 8/16/32 bits, scratchpad, BIOS de solo lectura, cache control.
- [x] Registros de I/O de todos los dispositivos; accesos sin dispositivo registrados en `Bus::unhandled_io`.
- [x] El bus posee la memoria y los dispositivos; RAM y BIOS tienen camino rápido.

## Fase 3 — Temporización, interrupciones y timers · HECHA salvo PAL/entrelazado fino

- [x] Interrupciones, timers 0/1/2 con sus fuentes de reloj, VBlank y hblank, bit par/impar de GPUSTAT.
- [x] Los dispositivos no avanzan en cada ciclo: el bus lleva la cuenta y los pone al día cuando llega su próximo evento (plazo calculado con cotas inferiores) o cuando la CPU toca un registro que depende del tiempo. El resultado es idéntico (comprobado con el checksum) y 2–3× más rápido.
- [x] Sincronización de los timers 0 y 1 con el hblank y el vblank (los cuatro modos); el GPU informa de los bordes del blanking y el bus no se salta ninguno mientras un timer está sincronizado.
- [ ] PAL y modo entrelazado fino.

## Fase 4 — Arranque de BIOS · HECHA

- [x] BIOS desde el host o el navegador (512 KB, validada), POST, logo SCE y PlayStation, arranque del juego.
- [x] Ejecutables PS-X EXE (`System::load_exe`): la BIOS arranca y el programa toma el relevo al llegar al shell.
- [x] Identificación de la BIOS: CRC-32 y línea de versión (versión, fecha, región); el modelo solo se nombra para sumas verificadas (hoy SCPH-1001). Se muestra en la web.
- [x] Traza de ejecución detrás del feature `trace` (`boot --features trace --trace N`).

## Fase 5 — GPU · HECHA salvo temporización

Archivos: `gpu/mod.rs`, `gpu/sw.rs`.

- [x] VRAM de 1024×512, GP0/GP1, GPUSTAT, transferencias, resoluciones y conversión a RGBA reutilizando el búfer.
- [x] Rasterizador: funciones de borde incrementales, tramos por fila, división exacta por recíproco y rellenos por filas. Se comprueba contra la implementación sencilla anterior con pruebas diferenciales aleatorias (triángulos, rectángulos, rellenos, copias).
- [x] El analizador de GP0 no reserva memoria por comando.
- [x] Temporización de la GPU: cada comando de dibujo deja la GPU ocupada un número estimado de ciclos (GPUSTAT 26 y 28), según el área, la textura y la mezcla. Es una estimación, no una medida de la máquina.
- [ ] Rellenos, líneas y polígonos exactos al píxel frente a una suite de referencia.

## Fase 6 — DMA · HECHA salvo temporización

- [x] Canales OTC, GPU, CD-ROM, SPU, MDEC in/out; bloque, slice y lista enlazada; las transferencias son instantáneas.
- [x] Duración de las transferencias (1 ciclo por palabra; SPU 4, CD-ROM 24, PIO 20): la CPU queda retenida y el canal sigue ocupado, con su interrupción, hasta que se acaba el tiempo.
- [x] Troceado (ventanas de DMA y de CPU) y canal PIO (sin dispositivo: lee `0xFFFFFFFF`).
- [ ] Prioridades entre canales: no hace falta mientras cada transferencia corre entera al empezar (ver «Decisiones»).

## Fase 7 — CD-ROM y formatos de disco

- [x] BIN/CUE (uno o varios archivos), ISO, sectores de 2048/2336/2352 bytes, lectura bajo demanda (archivo del host o función de JavaScript con `FileReaderSync`).
- [x] Controlador completo con retardos, velocidad simple/doble, GetID por región.
- [x] CD-DA (Play por pista), informes de posición (Setmode bit 2), fin de datos al acabar el audio, matriz de volumen.
- [x] XA-ADPCM de 4 y 8 bits, mono/estéreo, filtro de archivo/canal, remuestreo a 44,1 kHz.
- [x] Cambio de disco con la consola en marcha (el estado informa de la tapa abierta hasta el siguiente Getstat).
- [x] `PREGAP`/`POSTGAP` de los CUE (sectores silenciosos que desplazan las pistas), `INDEX 00` y subcanal Q con índice 0 y cuenta atrás en la pregap.
- [x] Forward/Backward durante el CD-DA y tiempos de búsqueda según la distancia.
- [x] Formatos: CloneCD (`.ccd` + `.img`) y SBI (LibCrypt: Q de los sectores que lista el archivo).
- [ ] PBP y CHD (ver «Decisiones»).

## Fase 8 — GTE · HECHA

- [x] Registros, todas las instrucciones, acumuladores de 44 bits, flags, tabla UNR generada con la fórmula documentada.
- [x] Probado indirectamente con Resident Evil (geometría 3D correcta).
- [x] Latencia en ciclos de cada comando; la CPU solo espera si toca el GTE antes de que acabe.
- [ ] Validación con una suite de pruebas del GTE de dominio público (no hay binarios verificados disponibles).

## Fase 9 — Entrada · HECHA salvo multitap

- [x] SIO0, pad digital, DualShock completo (modo analógico, configuración 0x43–0x4D, palancas, vibración con su mapeo), dos puertos, tarjetas de memoria.
- [x] SIO1: registros, FIFO de recepción, interrupciones, y salida/entrada accesibles desde el anfitrión.
- [ ] Multitap (ver «Decisiones»).

## Fase 10 — Audio (SPU) · HECHA salvo detalles

Archivos: `spu/` (mezclador, voces, reverb, kernel gaussiano), `xa.rs`.

- [x] ADPCM, ADSR, volúmenes con barrido, ruido, modulación de tono, reverb, IRQ, interpolación gaussiana con la tabla de la documentación (validada por su invariante: los cuatro pesos de cada fase suman lo mismo).
- [x] Buffers de captura (CD izquierdo y derecho, voces 1 y 3) en los primeros 4 KB de la RAM de sonido; el ancho de banda de las transferencias lo da la duración del DMA.
- [ ] Filtros FIR de entrada/salida del reverb (sin documentación verificable).
- [ ] Comprobar el audio de oídas en un juego (hoy solo se ha analizado su espectro).

## Fase 11 — MDEC · HECHA salvo temporización

- [x] Decodificación según la documentación: tabla de cuantización por posición de escaneo, caso de escala 0, IDCT con la tabla subida por el juego (13 bits, redondeo documentado), YUV→RGB, salida a 4/8/15/24 bits.
- [x] Canales DMA 0/1 con espera de datos.
- [ ] Decodificación temporizada y estado de bloque/FIFO exactos.

## Fase 12 — Frontend web · HECHA

Archivos: `web/` (`app.js` interfaz, `worker.js` emulador, `audio*.js`, `input.js`, `storage.js`, `settings.js`).

- [x] El emulador corre en un worker; dibuja en un `OffscreenCanvas` leyendo el cuadro de la memoria de WebAssembly sin copiarlo; el audio llega directo al hilo de audio.
- [x] Disco leído bajo demanda con `FileReaderSync` (649 MB sin cargarlos en memoria), BIOS recordada en IndexedDB, programas PS-EXE.
- [x] Teclado reconfigurable, mando estándar, palancas analógicas, botón Analog (F9 o menú rápido; Home abre el menú rápido), segundo mando en el puerto 2, vibración (Gamepad API).
- [x] Tarjetas de memoria persistentes (exportar, importar, formatear), tres ranuras de estados guardados, cambio de disco en caliente, ajustes (volumen, silencio, velocidad) y teclas guardados.
- [x] Probado en el navegador con el módulo real, la BIOS y el disco de `rom_test/`.
- [ ] Probar la persistencia de las tarjetas con una partida real y el audio con altavoces.
- [x] Documentación de usuario: [docs/guia-de-usuario.md](docs/guia-de-usuario.md).
- [x] Interfaz rediseñada: barra de menús, barra de herramientas, diálogo de ajustes, menú rápido a pantalla completa (teclado, ratón y mando), avisos en pantalla.
- [x] Mandos USB y Bluetooth: puerto, mapa de botones, zona muerta y prueba de vibración por mando.
- [x] Indicador de sonido y desbloqueo automático del audio.

## Fase 13 — Calidad de imagen y audio · HECHA

Archivos: `ps1-core/src/gpu/hires.rs`, `gpu/lane.rs`, `pgxp.rs`; `web/layout.js`, `overlay.js`, `audio*.js`.

- [x] **Volumen de la interfaz.** Fallaba porque `startAudio()` avisaba de forma síncrona, `refreshAudioChip` reescribía el deslizador con el valor guardado y `setVolume` leía después ese valor viejo. Ahora se lee el deslizador primero, la ganancia sigue una curva cuadrática y cambia suavizada (`setTargetAtTime`), y hay Re Pág / Av Pág.
- [x] **Sonido sin cortes.** Reserva de audio con interpolación lineal y *dynamic rate control* (±1 %), fundido de salida y entrada en lugar de corte seco, reserva automática que crece con los cortes y baja en calma, y cortes que no cuentan mientras el emulador está en pausa o a otra velocidad. El SPU en sí se comprobó grabando 98 s de Crash Bandicoot: sin saltos entre muestras, sin recorte y con silencios solo en las pausas de la música.
- [x] **Panel de rendimiento** (F2) con FPS, velocidad, tiempo por cuadro, carga, resolución, búfer de audio y memoria, cada uno a elección y en cuatro posiciones.
- [x] **Aspecto**: 4:3, 16:9, ampliar sin deformar (cubre la ventana), escala entera (múltiplo de las líneas de la consola, con la densidad de píxeles del equipo), rellenar, y recorte del borde negro. La aritmética está en `web/layout.js` con pruebas en Node.
- [x] **Escalado interno 1×/2×/3×** con una VRAM escalada de 8 bits por canal que se dibuja además de la de la consola: mismas reglas de bordes e interpolación exacta (pruebas diferenciales: píxeles idénticos a 1×, y idénticos en los puntos de muestreo de la consola a 2× y 3×), rellenos, subidas y copias reflejados.
- [x] **PGXP** (posiciones sin redondear de la GTE que siguen al valor por registros y memoria, validadas con el valor entero) y **texturas con perspectiva correcta**; un vértice saturado por la GTE (polígonos que cruzan el borde de la pantalla) conserva su posición precisa.
- [x] **Mejoras de calidad**: filtro bilineal con alfa binario y sin sangrado entre texturas, color verdadero de 24 bits, y escenas 3D ensanchadas para 16:9 (con PGXP).
- [x] **Menú rápido translúcido** con guardado y carga rápidos, ajustes rápidos y atajos de teclado que funcionan en pantalla completa.
- [x] **Calidad adaptativa** (`web/quality.js`): si el juego no llega a la velocidad real, el worker baja solo las opciones de imagen (bilineal, escalado, PGXP, color verdadero) y las recupera cuando hay holgura; se puede quitar en Gráficos. Probada con relojes inventados (`web/quality.test.mjs`) y en el navegador con Crash Bandicoot (3× + bilineal + PGXP + 24 bits al 27–55 % de velocidad → 100 % en ~9 s). Ver `docs/rendimiento.md`.
- [ ] Filtros de escalado de texturas tipo xBR/JINC2 y *scaled dithering*: no se han hecho; el escalado actual es resolución interna.
- [ ] Z-buffer de PGXP y el modo de CPU completo de PGXP (registros por todas las instrucciones): no hacen falta para que los vértices lleguen a la GPU en los juegos probados; el modo de memoria cubre `swc2`, `lw`/`sw` y `mfc2`.

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
| Interpolación por carriles (`gpu/lane.rs`) en el rasterizador de la consola | la multiplicación de 128 bits de `Divider` es una llamada a `__multi3` en WebAssembly (5 % del tiempo); cociente y resto se actualizan con sumas | ≈ 5 % en WebAssembly, igual en nativo |
| Bucles de fila planos en el rasterizador escalado | un `put` genérico por píxel costaba 3–4× más; especializados por tipo (plano, Gouraud, texturado, con mezcla, con perspectiva) y con la cuantización en una tabla | ×2–3 |

Medidas de la escena de juego de Crash Bandicoot (512×240) en WebAssembly con Node (el mismo motor que Chrome), ms por cuadro de emulación, portátil Core i3-8130U; varían entre tandas por la temperatura, por eso se dan intervalos:
nativo 8–11, 2× 15–22, 2× con PGXP 17–25, 3× con PGXP 26–40, 2× con filtro bilineal 27–42, 2× con color verdadero 19–22 (antes de optimizar los rasterizadores: 12, 22, 25, 39, 51 y 30). Con PGXP desactivado y 1× el estado de la máquina es idéntico bit a bit al de antes (mismo checksum del `bench` en la BIOS y en Crash Bandicoot) y la velocidad nativa es la misma. En el navegador, con el emulador en su worker, una escena repetible (estado guardado) dio 7,8 ms a 1×, 14,0 a 2× y 15,3 a 2× con PGXP.

Globalmente (nativo, mismo trabajo): 21 → 75 M instr/s en el arranque del juego y 11 → 65 M en
el arranque de la BIOS; WebAssembly 6 → 35 M y 10 → 45 M.

## Decisiones (descartado con una razón técnica)

- **SIMD de WebAssembly (`+simd128`)**: medido en ejecuciones alternadas, sin ganancia (la carga es de enteros escalares con saltos); pierde compatibilidad con navegadores antiguos.
- **Caché de bloques o recompilador**: el perfil muestra la CPU en ~80 % pero tras el camino rápido el coste por instrucción ya es de ~12 ns; un recompilador no puede emitir código nativo en WebAssembly y generar módulos al vuelo añade latencia y complejidad. Se reevaluará si hace falta más margen.
- **Backend WebGL2 para la GPU**: la GPU por software ocupa 4–8 % del tiempo tras optimizarla; un renderizador por hardware cambia la semántica de máscaras y transparencias y exigiría una suite de referencia que no hay. El escalado interno se hace por eso con la VRAM escalada por software (Fase 13), que sí se puede comprobar píxel a píxel contra el rasterizador de la consola.
- **Saltarse el dibujo a resolución nativa cuando hay escalado**: la VRAM nativa es de donde el juego lee texturas y lecturas de la GPU; derivarla de la escalada exigiría seguimiento de regiones sucias y no sería idéntica con PGXP, color verdadero o filtro, así que se dibuja en las dos.
- **Registros de I/O sin sincronizar dentro del camino rápido**: se implementó (las lecturas del BIOS al registro de retardo de la SPU eran 7 M de las 200 M de instrucciones del arranque) y se verificó idéntico, pero no dio ganancia medible en ejecuciones alternadas; se revirtió para no conservar complejidad sin beneficio.
- **Mapa de bits de validez de las etiquetas de PGXP, lectura de instrucciones por puntero a la página de código y IDCT del MDEC sin coeficientes a cero** (octubre de 2026): los tres se implementaron o se midieron en aislado y no dieron ganancia medible en los juegos (el segundo solo en bucles sintéticos, y pide `unsafe`); se revirtieron. Cifras en `docs/rendimiento.md`.
- **`wasm-opt`**: no está instalado y descargarlo no se justifica sin medir antes; el módulo ya se compila con LTO y `opt-level=3`.
- **Dibujo por campos en 480i / `draw_to_display`**: el emulador presenta cuadros completos; ignorar la restricción da la misma imagen final, solo cambia qué líneas se escriben a medio cuadro.
- **Lista de discos recientes**: los navegadores no permiten reabrir archivos locales sin un gesto del usuario, salvo con la File System Access API (solo Chromium).
- **FIR del reverb**: no hay descripción verificable en la documentación consultada; inventar coeficientes empeoraría el sonido.
- **ROMs de prueba de la CPU (`psxtest_cpu`, amidog)**: no se descargan ni ejecutan binarios de procedencia no verificada; se añadirán si el usuario los aporta en `rom_test/`.
- **PBP y CHD**: exigen decodificadores (zlib, LZMA, FLAC) y no hay archivos de muestra con los que comprobar el formato; un lector sin verificar daría imágenes corruptas sin avisar. CUE/BIN, ISO y CloneCD cubren los volcados habituales.
- **Multitap**: no hay una descripción del protocolo verificable ni un juego con el que probarlo.
- **Prioridades del DMA**: cada transferencia corre entera al empezar y retiene la CPU, así que nunca compiten dos canales.
- **Temporización del MDEC**: no hay constantes documentadas; el ritmo lo marca hoy la duración de los DMA de entrada y salida.
- **Memoria y caché de instrucciones**: ver Fase 1.
- **Comprobar el audio de oídas, la persistencia de las tarjetas y el sonido con altavoces**: requieren una persona con un juego y altavoces; solo se ha comprobado por espectro y con pruebas.

---

## Hitos de validación

1. **M1** — Los tests de CPU pasan. ✔
2. **M2** — La BIOS ejecuta sin errores durante varios cuadros. ✔
3. **M3** — Se ve la pantalla de arranque de la BIOS. ✔
4. **M4** — Arranca un juego de prueba con imagen y entrada. ✔
5. **M5** — Juego 3D con audio, FMV y partida jugable en el navegador. ✔ (Resident Evil; el audio se comprobó por espectro, no de oídas).
6. **M6** — Segundo juego: Crash Bandicoot (USA) llega al menú y, con `boot --press` (Start y Cruz), entra en la primera fase con Crash controlable. (parcial: solo en nativo con entrada simulada, ~3000 M de instrucciones, comprobado con capturas; sin probar en el navegador, con audio ni más allá de los primeros segundos de la fase).
7. **M7** — Tarjetas de memoria de punta a punta: la BIOS copia una partida de una tarjeta a otra y Resident Evil lee la tarjeta desde LOAD GAME sin `ACCESS ERROR`. ✔ (parcial: no se ha guardado una partida desde dentro de Resident Evil, solo con los servicios de la BIOS, ni se ha probado con una partida real en el navegador).
