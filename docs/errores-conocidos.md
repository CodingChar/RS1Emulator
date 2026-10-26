# Errores conocidos

## 1. «ACCESS ERROR» en la pantalla de tarjetas de Resident Evil (USA)

**Estado.** Resuelto. La causa estaba en el modelo de tiempos de la CPU, no en las tarjetas.

**Síntoma.** En la pantalla «MEMORY CARD 1» de Resident Evil (USA), al guardar o cargar, aparecía `ACCESS ERROR` aunque la tarjeta estuviera formateada. Se veía en el emulador web (Chrome/Edge) y se reprodujo igual en el arnés nativo.

**Causa.**
1. Al abrir la pantalla, el juego pide a la BIOS `_card_load`, que lee 36 sectores (cabecera, directorio y lista de sectores defectuosos). El núcleo de la BIOS atiende la tarjeta desde el manejador del blanking vertical y solo la mira una vez cada dos vblanks, así que cada sector tarda **2 fotogramas**: unos 41 millones de ciclos (1,2 s) para los 36.
2. Mientras tanto el juego espera los eventos de la tarjeta con un bucle de **249 999 iteraciones** (`0x3D08F`, cuatro `TestEvent` por vuelta), sin mirar el reloj. Si el bucle acaba antes de que llegue el evento, da error.
3. El modelo de la CPU cobraba **1 ciclo por instrucción**, también para las lecturas de RAM. Cada vuelta costaba ~172 ciclos y el bucle duraba ~40,6 millones: 1 % menos de lo que necesitaba la BIOS. El juego se rendía tras leer 33 de los 36 sectores. En la consola real una lectura de RAM tarda unos 6 ciclos (la R3000A de la PS1 no tiene caché de datos); con esa latencia cada vuelta cuesta ~231 ciclos y el bucle dura ~57,7 millones (1,7 s), un 40 % más de lo que necesita la BIOS.

**Cómo se comprobó.** Con `boot --card` y `--press` se llevó el juego a LOAD GAME en nativo y se reprodujo el error. Con `--bios-call` se vio que la BIOS leía los sectores 0 a 32 y se paraba, que el juego volvía a pedir `_card_load` justo cuando agotaba el bucle y que el tiempo entre sectores no dependía de la velocidad del puerto serie (se probó con el puerto 4× más rápido). Al cobrar 5 ciclos por cada lectura de RAM, la pantalla pasa por «CHECKING MEMORY CARD» y muestra «THERE IS NO FILE».

**Arreglo.** `RAM_READ_CYCLES = 5` (`cpu/interpreter.rs`): cada lectura de RAM cuesta 5 ciclos más que el ciclo de la instrucción, en el camino exacto y en el rápido (`cpu/interpreter/fast.rs`). La multiplicación, la división y el GTE siguen trabajando durante la espera. No se cobran escrituras (buffer de escritura), búsquedas de instrucciones (caché de instrucciones no modelada), BIOS ni registros de E/S.

**Pruebas.**
- `cpu/interpreter/tests.rs`: coste de cada tipo de carga, de escrituras, del scratchpad, de los espejos de RAM, de lecturas que fallan, solapamiento con `mult` y equivalencia entre el camino rápido y el exacto.
- `tests/boot_games.rs` (ignorado, necesita BIOS y disco): Resident Evil lee los 72 sectores de la tarjeta (36 al arrancar y 36 en LOAD GAME). Sin el arreglo lee 69 y falla.
- `tests/memory_card_bios.rs` (ignorado, necesita BIOS): el gestor de tarjetas de la BIOS copia una partida de la tarjeta 1 a la 2 con sus propios servicios de lectura y escritura. Si el núcleo descartara las escrituras, falla.
- `controllers/`: lectura y escritura de los 1024 sectores, errores de checksum y de sector, sesiones cortadas, reinicio de la tarjeta y escritura a través de los registros del puerto serie.

**Hipótesis descartadas.** La tarjeta 1 sí se formatea y se guarda (IndexedDB) y el protocolo de lectura y escritura sigue la especificación y el comportamiento de DuckStation; el fallo no dependía de ellos.

**Cambios relacionados en las tarjetas.**
- Al reiniciar la consola las tarjetas vuelven a anunciarse como nuevas (bit 3 del estado), como al encender la consola.
- Una escritura rechazada (checksum o sector erróneos) activa el bit 2 del estado, y una buena lo borra.
- Se aceptan imágenes `.gme` (DexDrive) y `.vmp` (PSP/PS3) además del volcado crudo de 128 KB; se guarda siempre el volcado crudo.
- La página avisa si no puede guardar una tarjeta en el navegador (y lo reintenta) y guarda también al ocultarse la pestaña.

**Sin comprobar.** Guardar una partida desde dentro de Resident Evil (hace falta jugar hasta una máquina de escribir y tener cinta de tinta). Lo que sí se comprobó: el juego lee la tarjeta desde LOAD GAME y la BIOS escribe y copia archivos con los mismos servicios de escritura que usan los juegos. Tampoco se ha probado con una partida real en el navegador; en la página se probaron la importación y el rechazo de archivos, y las funciones nuevas del módulo WebAssembly.

## 2. Limitaciones de las opciones de imagen (escalado, PGXP, ensanchado)

**Estado.** Conocidas; no son fallos de la emulación (con todo desactivado la imagen es la de la consola, píxel a píxel).

- **Coste.** El escalado dibuja `escala²` veces más píxeles: en la escena de juego de Crash Bandicoot (512×240) medida en WebAssembly con Node, un cuadro pasa de 8–11 ms (nativo) a 15–22 ms (2×) y 26–40 ms (3× con PGXP); el filtro bilineal es el más caro (27–42 ms a 2×). En un portátil modesto 2× llega a la velocidad real (16,7 ms por cuadro) en escenas ligeras y 3× o el bilineal necesitan un equipo más rápido. El panel de rendimiento (F2) lo muestra.
- **Ensanchar escenas 3D (16:9).** Solo se estrechan los polígonos cuyos vértices PGXP conoce todos; los elementos 2D se estiran, y los juegos que no envían a la GPU los polígonos fuera de la vista 4:3 dejan huecos en los lados (el fondo se ve donde antes no había nada que dibujar). Es la misma limitación que tiene el truco panorámico de DuckStation.
- **PGXP en el modo de memoria.** La posición precisa sigue al valor por `swc2`, `mfc2`, `lw`, `sw`, `lwc2` y `mtc2`; si un juego recalcula las posiciones con la CPU (sumas, desplazamientos) antes de entregarlas a la GPU, esos vértices no tienen etiqueta y se dibujan con enteros. Los estados guardados no llevan las etiquetas: tras cargar uno, lo que ya estaba preparado se dibuja con enteros hasta el siguiente cuadro.
- **Resolución y lecturas de VRAM.** El juego sigue viendo la VRAM nativa: un juego que lea la imagen de la pantalla para hacer efectos (o que la use como textura) ve la versión de la consola, y lo que vuelva a escribir en la VRAM se repite `escala × escala` veces en la imagen escalada.
- **Audio.** El sonido que produce el SPU se comprobó sin saltos ni recortes (98 s de Crash Bandicoot); lo que cambia es el reproductor del navegador. Si el equipo no alcanza la velocidad real, el sonido se corta aunque la reserva sea grande: no es un fallo del audio sino de la velocidad.
