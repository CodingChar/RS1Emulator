# Rendimiento

Cómo se midió y qué se cambió para que Silent Hill (y cualquier juego con bucles de espera a la GPU) vaya a velocidad real en equipos modestos.

## Cómo se mide

- **WebAssembly en Node** (el mismo motor V8 que Chrome y Edge): `wasm-bindgen --target nodejs`, se carga un estado guardado de Silent Hill dentro de una escena y se cronometra `run_frame` y `render_frame` fotograma a fotograma (media, mediana, percentiles 95 y 99 y máximo). `node --cpu-prof` da el tiempo propio de cada función. El portátil de pruebas tiene un ruido del ±15 % (a ratos mucho más): se alternan las versiones y se compara el mejor caso.
- **Nativo**: `examples/bench.rs` ejecuta el juego y da un *checksum* del estado completo de la máquina (pantalla, registros, RAM, audio). **Una optimización no puede cambiar ese checksum**: debe ser idéntico antes y después en Crash Bandicoot, Silent Hill y Resident Evil (con las mismas pulsaciones). Así se comprueba que el emulador hace exactamente lo mismo, solo que antes.

## Dónde se iba el tiempo (Silent Hill, escena de juego, 1×)

Antes: 12–14 ms por fotograma de media, 22–26 ms en el percentil 95 y picos de 45–52 ms (el congelado de la imagen). En un equipo más lento, 21 ms de media: 74 % de velocidad.

| Función | Tiempo |
|---|---|
| Intérprete de la CPU (`run_fast` y el camino lento) | 59 % |
| Dispositivos (`Gpu::advance`, `Timers::tick`, `skip_devices`, `Spu::step`, DMA) | 17 % |
| Rasterizador de la GPU | 8 % |

Contadores temporales dieron la causa: por fotograma había ~47 000 lotes rápidos de solo 8 instrucciones y ~46 000 pasos por el camino lento, **todos lecturas de `GPUSTAT`** (`0x1F801814`, 43 600 de ellas desde un único bucle de cinco instrucciones, `0x8001A3D8`). El juego espera a que la GPU esté libre, y cada lectura sacaba a la CPU del camino rápido, volvía a decodificar la instrucción y sincronizaba los seis dispositivos. Ese bucle ocupaba más de la mitad de las instrucciones ejecutadas.

## Qué se cambió

1. **`GPUSTAT` se lee dentro del camino rápido** (`Gpu::status_after`, `Bus::gpu_status_after`). El valor que daría la lectura exacta se calcula proyectando el estado de la GPU hacia delante los ciclos pendientes, sin mover nada: solo el tiempo ocupado, el campo y la línea cambian con el tiempo. Sin dejar el lote ni sincronizar dispositivos. Silent Hill 12,2 → 8 ms (mejor caso), Crash Bandicoot +81 % en nativo.
2. **Los turnos de espera a la GPU se saltan** (`poll_loop`, `skippable_polls` en `cpu/interpreter/fast.rs`). El bucle `lw / nop / and / beq / nop` se reconoce de forma estricta y, cuando ya ha dado una vuelta, se saltan de golpe las vueltas que leerían una GPU todavía ocupada: se sabe exactamente cuándo termina (`Gpu::cycles_until_idle`) y el estado tras saltarlas es el mismo que tras ejecutarlas (el registro ya vale 0; solo pasa el tiempo). Máscaras que ven los bits que siguen a la exploración (13 y 31) no se saltan. El lote termina donde lo habrían terminado las vueltas saltadas. Silent Hill 8 → ~5,6 ms.
3. **Ritmo del bucle** (`web/worker.js`): se recupera como mucho de 2 en 2 fotogramas y la deuda restante se descarta, en lugar de acumular hasta 4 y ejecutarlos de golpe antes de dibujar (la imagen se quedaba quieta y luego saltaba). Un equipo que no llega a la velocidad real enseña todos los fotogramas, a la velocidad que alcanza. Los temporizadores se encadenan por un `MessageChannel` para esquivar el mínimo de 4 ms de los temporizadores anidados.
4. **Presentación con WebGL2** (`web/present.js`): la imagen sube a la GPU como textura directamente desde la memoria del emulador. `putImageData` copia y convierte antes (en la captura de un portátil modesto eran 3,7 ms por cuadro). Si el navegador no da WebGL2, se usa el canvas 2D de antes; `?canvas=2d` en la dirección fuerza el 2D para comparar.

## Pruebas

- `gpu/tests.rs`: `status_after_is_what_advancing_then_reading_gives` y `cycles_until_idle_is_the_first_cycle_the_gpu_is_ready` (propiedades sobre estados aleatorios).
- `cpu/interpreter/tests.rs`: `polling_gpustat_gives_the_same_values_on_the_fast_and_the_exact_path` y `the_wait_for_the_gpu_loop_is_skipped_without_changing_anything` comparan el camino rápido con el exacto, instrucción a instrucción y ciclo a ciclo (incluidos los ciclos que dura cada ejecución). Se comprobó que ambos fallan si se desplaza el tiempo unos ciclos.
- Los *checksums* de estado de los tres juegos son idénticos a los de antes.

## Probado y descartado

- **SIMD (`+simd128`)**: sin mejora medible, ni a 1× ni a 3×, y excluye navegadores antiguos.
- **`wasm-opt -O3`**: el módulo baja de 491 a 381 KB, pero la velocidad no cambia dentro del ruido. No compensa el paso extra en el despliegue.
- **RAM como matriz de tamaño fijo** (para quitar las comprobaciones de límites): sin diferencia medible.
- `opt-level = 3`, `lto = true`, `codegen-units = 1` y `panic = "abort"` ya estaban. `rustc` actual ya activa `bulk-memory`, `sign-ext` y `nontrapping-fptoint` en `wasm32-unknown-unknown`.

## Lo que queda

Con la espera a la GPU resuelta, el intérprete sigue siendo lo más grande (~55 %). Quedan lecturas de `JOY_STAT` (`0x1F801044`, ~2 000 por fotograma), `I_STAT` y los contadores, que aún salen del camino rápido, y los picos de los fotogramas con polígonos enormes (niebla en dos pasadas), que son trabajo real del rasterizador.
