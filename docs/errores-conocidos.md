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

## 3. Silent Hill (USA) se quedaba en la pantalla de aviso

**Estado.** Resuelto. La causa estaba en el controlador del CD-ROM, no en el juego ni en la carga del disco.

**Síntoma.** Con el disco de Silent Hill (USA) el emulador arrancaba, mostraba «There are violent and disturbing images in this game» y no pasaba de ahí (ni en el navegador ni en el arnés nativo, con o sin tarjeta de memoria). No hay fallo ni pánico: el juego sigue ejecutando su bucle a 60 cuadros por segundo.

**Causa.**
1. Durante la pantalla de aviso el juego transmite desde el disco el sonido de fondo (lecturas de 25 sectores con `CdRead`, modo doble velocidad y sector completo de 2340 bytes). Mientras espera a la GPU entre dibujos, enmascara todas las interrupciones (`I_MASK = 0`), a veces durante más de un sector (225 792 ciclos a doble velocidad).
2. El manejador de la interrupción del CD-ROM primero **reconoce** la interrupción y solo unos 300 ciclos después pide los datos (BFRD) y lee la cabecera de 12 bytes. Nuestro controlador entregaba el siguiente sector **en el mismo ciclo del reconocimiento** si ya le tocaba, así que el búfer ya contenía el sector N+1 cuando el juego pedía el N.
3. `libcd` compara el minuto/segundo/cuadro de la cabecera con el sector que espera. Al no coincidir marca error y repite toda la lectura desde el principio (Pause, Setloc, SetMode, ReadN). Eso se repetía unas 330 veces hasta que a los 1200 cuadros (20 s) `CdReadSync` se rendía y devolvía `-1`.
4. El controlador de sonido del juego solo espera a que `CdReadSync` devuelva 0 y trata cualquier otro valor como «ocupado», de modo que no volvía a intentarlo y la pantalla de aviso quedaba para siempre.

**Cómo se comprobó.** Se siguió el juego desde el bucle de la pantalla hasta `CdReadSync` desensamblando su código y vigilando la variable que bloquea al controlador de sonido (`0x800C166D`). Un seguimiento de los registros del CD-ROM mostró dos INT1 seguidos sin petición de datos entre ellos, con el reconocimiento llegando 228 204 ciclos después del INT1 (más que el período de un sector) y las cabeceras saltándose un sector (`0x28` seguido de `0x30`).

**Arreglo.** Tras reconocer una interrupción, el controlador espera `ACK_TO_NEXT_INTERRUPT = 5000` ciclos antes de lanzar la siguiente (sector o respuesta en cola), lo bastante para que el manejador pida y lea los datos del sector anunciado (`cdrom.rs`). El estado guardado cambia (`SAVE_STATE_VERSION = 9`).

**Pruebas.**
- `cdrom.rs`: `a_late_acknowledge_still_gets_the_data_of_the_announced_sector` (falla sin el arreglo).
- `tests/boot_games.rs` (ignorado, necesita BIOS y disco): `silent_hill_leaves_the_warning_screen_for_its_intro_video`. Sin el arreglo emite 1255 órdenes al CD; con él, menos de 200 y llega al vídeo de introducción.

**Pendiente.** La estimación del tiempo de dibujo de la GPU (`gpu/mod.rs`, `cost`) cobra 2 relojes por píxel a los sprites texturizados; si es demasiado pesimista, los juegos pasan más tiempo con las interrupciones enmascaradas que en la consola real. No se tocó: con el arreglo del CD el juego funciona y los cambios en esa estimación afectarían a la temporización de otros juegos.

## 4. Silent Hill (USA): gráficos rotos al jugar (recuadro alrededor de Harry, texturas borradas, líneas basura)

**Estado.** Resuelto. Eran tres fallos distintos, dos de la GPU y uno de la CPU; afectaban a cualquier juego que use esas funciones, no solo a Silent Hill.

### 4.1 El bit 15 de la textura se perdía (recuadro claro alrededor de los personajes)

**Síntoma.** En la demo del título y en la partida, Harry aparecía dentro de un rectángulo más claro, y en otras escenas como una silueta oscura.

**Causa.** Silent Hill aplica la niebla en dos pasadas: dibuja el polígono texturizado y encima otro polígono semitransparente del color de la niebla con «set mask + check mask» (`E6h = 3`), de modo que donde dos polígonos se solapan la niebla no se suma dos veces. Para que eso funcione, los píxeles que escribe un polígono **texturizado** conservan en la VRAM el bit 15 del texel (STP), que es el que mira después el «check mask». Nosotros solo escribíamos el bit 15 cuando el juego activaba «set mask», así que el recuadro de niebla del personaje se aplicaba también sobre el fondo ya pintado.

**Arreglo.** Un píxel texturizado se guarda con el bit 15 del texel además del «set mask» (`gpu/sw.rs`, `gpu/sw/reference.rs` y `gpu/hires.rs`, en todos los caminos de dibujo: triángulos y rectángulos, en el renderizador exacto, en el rápido y en el escalado). Los primitivos sin textura siguen escribiendo solo el bit de «set mask».

**Pruebas.** `gpu/sw/tests.rs` (`textured_pixels_keep_the_texel_bit_15_for_a_later_mask_check`, `a_blended_texel_keeps_bit_15_and_an_opaque_one_does_not_get_it`) y `gpu/hires/tests.rs` (la misma comprobación en el renderizador escalado). Las pruebas de equivalencia entre el renderizador exacto, el rápido y el escalado a 1× siguen pasando.

### 4.2 La imagen mostraba líneas fuera del rango vertical (líneas basura arriba o abajo)

**Síntoma.** Algunos fotogramas traían una o dos líneas de píxeles de colores arriba o franjas rosas abajo.

**Causa.** El emulador mostraba siempre 240 líneas desde el inicio del área de visualización y no hacía caso del rango vertical (`GP1(07h)`). Un juego que dibuja 224 líneas (Silent Hill usa el rango `0x18..0xF8`) deja debajo de su imagen datos de otras cosas en la VRAM, y la consola no los muestra: las líneas fuera del rango salen en negro.

**Arreglo.** `Gpu::visible_lines` calcula qué líneas de la imagen enseñan la VRAM: la imagen empieza en el scanline 16 (35 en PAL), las líneas antes y después del rango se pintan negras y la primera línea del rango muestra la primera fila del área. Vale para la imagen de la consola, la escalada y los modos de 480 líneas. Con el rango por defecto no cambia nada.

**Pruebas.** `gpu/tests.rs`: `the_vertical_display_range_blanks_the_lines_outside_it`, `a_range_that_starts_before_the_picture_skips_the_first_rows`, `an_empty_range_shows_nothing_and_the_default_shows_everything` y `the_scaled_picture_is_cropped_like_the_console_picture`.

### 4.3 Un comando de la GTE bajo una interrupción se perdía (texturas borradas, GPU desincronizada)

**Síntoma.** Al empezar una partida nueva la escena se veía casi sin texturas (suelo verde, siluetas planas) y se quedaba así.

**Causa.**
1. La instrucción `DPCS` de la GTE (niebla por profundidad) estaba seguida de `swc2` para guardar el color en el paquete de la GPU. Cuando una interrupción caía justo sobre el `DPCS`, nosotros tomábamos la excepción **antes** de ejecutarlo, con `EPC` apuntando a él.
2. El manejador de excepciones de la BIOS (en `0xC80`) mira la instrucción que hay en `EPC` y, si es un comando de la GTE, **vuelve a la siguiente**: en la consola real la CPU ya ha lanzado el comando cuando llega la excepción. Resultado: el `DPCS` no se ejecutaba nunca y `swc2` guardaba el color de un cálculo anterior.
3. Ese color traía el byte de código en 0 (`02030303` en lugar de `3E030303`). La GPU tomó el paquete por un comando de relleno (`02h`) y trató las palabras del polígono como más comandos: miles de rellenos con coordenadas sin sentido que borraron las texturas de la VRAM.

**Cómo se comprobó.** Un registro de los comandos GP0 mostró el primer `02030303` en medio de una lista de polígonos; la dirección del paquete en la RAM y un vigilante de la palabra en el bloc de notas de la CPU dieron la instrucción responsable, y el rastro de instrucciones (`--features trace`) mostró la excepción entre un `nop` y el `DPCS`, y la vuelta directa al `swc2`.

**Arreglo.** Si hay una interrupción pendiente y la instrucción que iba a ejecutarse es un comando de la GTE (y no está en un hueco de salto, donde `EPC` apunta al salto), se ejecuta el comando antes de entrar en el manejador (`cpu/interpreter.rs`, `run_gte_command_under_interrupt`). Es lo que hace el hardware y lo que hace DuckStation.

**Pruebas.** `cpu/interpreter/tests.rs`: `an_interrupt_on_a_gte_command_still_runs_the_command`, `an_interrupt_on_other_instructions_runs_no_gte_command` y `an_interrupt_on_a_gte_command_in_a_delay_slot_runs_it_once_after_the_return`.
