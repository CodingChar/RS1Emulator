# RS1Emu — Guía de usuario

RS1Emu emula la PlayStation 1 en el navegador. Todo ocurre en tu equipo: la BIOS, los
discos y las partidas no se envían a ningún servidor.

## Qué necesitas

- **Una BIOS de PS1**: el volcado de 512 KB de tu propia consola (por ejemplo `SCPH1001.BIN`).
  El emulador la reconoce por su suma de comprobación y por la línea de versión que lleva dentro,
  y la recuerda en este navegador.
- **Un disco** (opcional): una copia de seguridad de uno de tus juegos. Se aceptan:
  - `.cue` con sus `.bin` (selecciónalos todos a la vez),
  - un `.bin` o `.iso` suelto,
  - `.ccd` con su `.img` (CloneCD),
  - un programa PS-EXE (`.exe`, `.psx`),
  - opcionalmente un `.sbi` junto al disco (datos de subcanal de algunos discos europeos).

Sin disco, **Iniciar** abre el menú de la BIOS (tarjetas de memoria y reproductor de CD).

## Empezar

1. Abre la página y elige la BIOS (botón *Elegir…* o arrástrala a la ventana).
2. Elige el disco, o arrastra los archivos a la ventana.
3. El juego arranca solo. Si no, pulsa **Iniciar**.

Los discos se leen a medida que hacen falta, así que uno de 650 MB no consume memoria.

## La ventana

- **Barra de menús**: Sistema (abrir disco, cambiar disco, estados), Ver, Ajustes y Ayuda.
- **Barra de herramientas**: iniciar, pausar, reiniciar, cambiar disco, guardar y cargar estado,
  sonido, menú rápido, pantalla completa y ajustes.
- **Barra de estado**: fotogramas por segundo y porcentaje de la velocidad real, velocidad
  elegida, modo analógico y estado del sonido.

## Menú rápido

Se abre con **Esc**, **F1** o el botón **Home** del mando, y también desde la barra de
herramientas. Pausa el juego mientras está abierto, se ve a través de él (el juego queda
detrás, desenfocado) y no deja pasar ningún botón al juego. Se maneja con las flechas y Intro,
con el mando (cruceta, ✕ para elegir, ○ para volver) o con el ratón. Arriba está lo que más
se usa:

- **Reanudar**,
- **Guardado rápido y carga rápida** (F5 y F7) de la ranura elegida: ◀ ▶ cambian de ranura sin salir del menú,
- **ajustes rápidos**: volumen, relación de aspecto, escalado, PGXP, panel de rendimiento, velocidad y modo analógico,
- páginas completas de **Pantalla y sonido**, **Gráficos** y **Panel de rendimiento**,
- pantalla completa, cambiar el disco con el juego en marcha (los juegos de varios discos lo piden), reiniciar y ajustes.

En pantalla completa, **Esc** abre el menú rápido (mantenlo pulsado para salir de la pantalla
completa; si tu navegador no lo permite, usa **F1**). **Alt + Intro** alterna la pantalla completa.

### Atajos de teclado

Funcionan con el juego en marcha, en ventana y en pantalla completa:

| Tecla | Acción |
|---|---|
| Esc · F1 · Home (mando) | menú rápido |
| F5 / F7 | guardado y carga rápidos de la ranura elegida |
| F6 | siguiente ranura de estado |
| F2 | mostrar u ocultar el panel de rendimiento |
| F3 | cambiar la relación de aspecto |
| F4 | cambiar el escalado (1×, 2×, 3×) |
| F8 | silenciar o activar el sonido |
| F9 | modo analógico del mando 1 |
| F10 | pausar o reanudar |
| Re Pág / Av Pág | subir o bajar el volumen |
| Alt + Intro | pantalla completa |

Cada atajo muestra un aviso en pantalla con lo que ha hecho.

## Controles

### Teclado (siempre el mando 1)

| Botón | Tecla | Botón | Tecla |
|---|---|---|---|
| Cruceta | Flechas | ✕ | Z |
| ○ | X | □ | A |
| △ | S | L1 / R1 | Q / W |
| L2 / R2 | E / R | Start | Intro |
| Select | Shift derecho | Analógico | F9 |

Se pueden cambiar en *Ajustes → Teclado*.

### Mandos USB y Bluetooth

Conecta el mando por USB, o empareja el de Bluetooth en el sistema operativo, y **pulsa un botón**:
el navegador no lo ve hasta entonces. Funcionan DualShock, DualSense, Xbox, Switch Pro, 8BitDo y
la mayoría de los genéricos. En *Ajustes → Mandos* cada mando tiene:

- el **puerto** al que va (automático: el primero al puerto 1 y el segundo al 2),
- la **zona muerta** de las palancas,
- una **prueba de vibración**,
- la **asignación de botones y palancas**: pulsa un control de la lista y luego el botón del
  mando. Sirve para los mandos genéricos cuyo mapa no es estándar y para cruces que el navegador
  presenta como ejes. Se guarda por modelo de mando.

Mientras mantienes pulsado un botón verás en esa pantalla cuál reconoce el emulador.

El modo analógico (la luz roja del DualShock) se cambia con **F9** o desde el menú rápido.
La vibración funciona si el navegador y el mando la permiten.

## Partidas guardadas

- **Tarjetas de memoria**: las dos se guardan solas en este navegador (al guardar en el juego, al
  cabo de un segundo como mucho, y al cambiar de pestaña o cerrarla). En *Ajustes → Tarjetas de
  memoria* se pueden exportar (`.mcr`), importar (`.mcr`, `.mcd`, `.bin`, `.gme` de DexDrive o
  `.vmp` de PSP/PS3) y formatear. Si el navegador no puede guardar una tarjeta, la página lo avisa
  y lo reintenta: exporta la tarjeta para no perder la partida.
- **Estados guardados**: tres ranuras (**F5** guarda, **F7** carga la ranura elegida). Guardan toda
  la máquina, así que carga el mismo disco antes de restaurar. Un estado solo vale para la misma
  BIOS y la misma versión de RS1Emu.

Los datos viven en el almacenamiento del navegador: si lo borras, se pierden. Exporta las
tarjetas que te importen.

## Sonido

El navegador no deja sonar el audio hasta que interactúas con la página. El indicador de la
barra de estado dice en qué punto está:

- **Sonido activo**: todo bien.
- **Sonido en espera**: haz clic en el indicador (o en cualquier parte de la página).
- **Silenciado**: pulsa el altavoz de la barra de herramientas.
- **Sin sonido a esta velocidad**: con velocidad ×2, ×4 o máxima no se reproduce audio.

El **volumen** se cambia con el deslizador de la barra de herramientas, en *Ajustes → Audio*,
en el menú rápido o con Re Pág / Av Pág; se recuerda. Es proporcional a lo que se oye (el
deslizador no es lineal: el 50 % suena a medias, no a un cuarto).

### Cortes y chasquidos

El emulador produce el sonido en ráfagas, un fotograma cada vez, y la tarjeta de sonido lo
consume con su propio reloj. Para que no se corte, el reproductor guarda una **reserva** de sonido
y la mantiene estable: acelera o frena su reproducción como mucho un 1 % (no se oye) según se llene
o se vacíe, y si aun así se vacía hace un fundido en lugar de cortar en seco y espera a que vuelva
a llenarse. En *Ajustes → Audio → Reserva de sonido* se elige:

- **Automática** (por defecto): empieza en 80 ms, crece sola cuando el sonido se corta y vuelve a
  bajar tras unos segundos sin cortes.
- **40, 70, 120 o 200 ms** fijos: más reserva es más seguro en un equipo lento; menos, más inmediato.

Si el sonido se corta aunque la reserva sea grande, el equipo no llega a la velocidad real del juego:
mira el panel de rendimiento (carga y porcentaje) y baja el escalado.

## Pantalla

*Ajustes → Pantalla* (o *Ver* y el menú rápido):

- **Relación de aspecto**:
  - *4:3 (original)*: como la consola.
  - *16:9 (panorámica)*: estira la imagen a una pantalla panorámica. Para que las escenas 3D no se
    vean estiradas, activa *Ensanchar escenas 3D* en *Gráficos* (necesita PGXP).
  - *Ampliar sin deformar*: la imagen 4:3 llena toda la ventana o pantalla sin estirarse; recorta un
    poco por arriba y por abajo en una pantalla panorámica.
  - *Rellenar la ventana*: llena la ventana estirando la imagen.
  - *Escala entera*: un número entero de píxeles del equipo por cada línea de la consola (píxeles
    perfectos, con bandas negras).
- **Recortar el borde negro** (poco, medio, mucho): la imagen de muchos juegos trae un borde negro; al
  recortarlo la imagen ocupa más pantalla sin deformarse.
- **Filtro de la imagen**: píxeles nítidos o suavizado.

## Gráficos

*Ajustes → Gráficos* cambia cómo dibuja el emulador; lo que ve el juego no cambia, y con todo en
los valores predeterminados la imagen es la de la consola, píxel a píxel.

- **Escalado (resolución interna y texturas)**: *Nativo* (1×, predeterminado), *2×* o *3×*. Dibuja la
  imagen con 2 o 3 veces más píxeles en cada dirección, y las texturas con ella. La geometría gana
  definición (bordes finos, sin escalones), las texturas se ven más grandes y nítidas, y el texto y los menús
  en 2D siguen igual de limpios porque los sprites no se filtran.
- **Filtro de texturas**: *sin filtro* (los píxeles de la consola) o *bilineal*, que mezcla los 4 texels
  vecinos (los transparentes no tiñen los bordes y la mezcla no se sale de la textura del polígono).
- **PGXP: geometría de precisión**: la GTE de la consola redondea cada vértice a píxeles enteros, y de ahí
  viene el temblor de los polígonos y la deformación de las texturas. PGXP (la técnica de Beetle PSX y de
  DuckStation) conserva la posición sin redondear que la GTE calculó y la usa para dibujar. Se nota
  sobre todo con 2× o 3×. Incluye:
  - **Texturas con perspectiva correcta**: la consola mapea las texturas sin perspectiva (se deforman en
    suelos y paredes inclinados); con esta opción se interpola con la profundidad de cada vértice.
  - **Ensanchar escenas 3D (16:9)**: comprime horizontalmente la geometría 3D (3/4 respecto del centro) para
    que, mostrada a 16:9, no se vea estirada. Los elementos 2D (menús, marcadores) se estiran, y los juegos que
    no dibujan los polígonos fuera de la pantalla 4:3 dejan huecos en los lados.
- **Color verdadero (24 bits)**: dibuja y mezcla con 8 bits por canal, sin el recorte a 15 bits ni el
  tramado de la consola (más degradados suaves, sin patrón de puntos).
- **Calidad adaptativa** (activada de serie): si el juego no llega a la velocidad real, el emulador baja
  solo lo más caro, de un escalón en un escalón, y lo recupera cuando hay holgura. El orden es: filtro
  bilineal, escalado (3× → 2× → 1×), PGXP y color verdadero; con *Ensanchar escenas 3D* activo no se
  quita PGXP, porque sin él la imagen solo se estiraría. Avisa con un mensaje cada vez que cambia algo y
  el panel de rendimiento muestra «calidad adaptada» mientras haya algo reducido. Mira el tiempo que
  tarda cada cuadro durante un par de segundos antes de decidir, no se mueve por un tirón aislado, y
  espera como mínimo 15 s antes de volver a subir un escalón (el doble si el escalón no aguantó). Solo
  actúa a velocidad normal o más lenta: con ×2, ×4 o máxima se supone que pides todo lo que dé el
  equipo. Cambiar cualquier ajuste de imagen la reinicia con lo que hayas elegido.

Cada mejora cuesta tiempo: con 2× son 4 veces más píxeles y con 3× son 9 veces más; el filtro bilineal
es el más caro. Si el juego va más lento que la consola (el porcentaje de la barra de estado baja de
100 %) y has quitado la calidad adaptativa, baja el escalado, quita el filtro bilineal o desactiva
PGXP a mano. Los estados guardados no incluyen
la información de PGXP: tras cargar uno, los polígonos que ya estaban preparados se dibujan con posiciones
enteras hasta el cuadro siguiente.

## Panel de rendimiento

**F2** (o *Ajustes → Panel de rendimiento*) muestra sobre la imagen las cifras que tú elijas:

- **FPS**: cuadros por segundo del juego,
- **Velocidad de emulación**: porcentaje respecto de la consola real,
- **Tiempo por cuadro**: lo que tardan la emulación y el dibujo, y el mayor intervalo entre cuadros,
- **Carga del procesador**: porcentaje del tiempo de cada cuadro que se gasta (por encima de 100 % no llega),
- **Resolución y renderizado**: tamaño de la imagen de la consola y de la que se dibuja, escalado, PGXP, filtro,
- **Búfer de audio**: sonido en reserva, objetivo y cortes,
- **Memoria** del emulador.

Se puede colocar en cualquiera de las cuatro esquinas. Las cifras que se salen de lo normal se ponen
en amarillo o rojo. Con el panel oculto no cuesta nada.

## Velocidad

Normal, ×2, ×4 o máxima, desde el menú rápido o *Ajustes → General*. En un equipo lento el
porcentaje de la barra de estado baja de 100 %: el juego va más despacio que la consola.

## Problemas frecuentes

| Síntoma | Qué probar |
|---|---|
| «BIOS no válida» | Debe ser un archivo de exactamente 512 KB. |
| «Falta el archivo .bin» | Selecciona el `.cue` y todos sus `.bin` a la vez. |
| El juego no arranca | Comprueba que el disco es de PS1 y que la BIOS es de la misma región o compatible. |
| No suena | Mira el indicador de sonido (arriba) y que la velocidad sea normal. |
| El mando no aparece | Pulsa un botón con la página en primer plano; prueba otro puerto USB. |
| Va a saltos | Deja activada la *Calidad adaptativa* (Gráficos), que baja sola el escalado, PGXP o el filtro bilineal; si la tienes quitada, hazlo a mano. También ayuda bajar la velocidad objetivo o cerrar otras pestañas pesadas. |
| El sonido se corta | Pon la reserva de sonido en Automática o más alta, y mira en el panel de rendimiento si la carga pasa de 100 %. |
| La imagen se ve estirada | Elige *Ampliar sin deformar* o *Escala entera* en Pantalla; con 16:9, activa *Ensanchar escenas 3D*. |
| Una tarjeta se perdió | El navegador pudo borrar sus datos; importa una copia exportada. |

## Para quien desarrolla

- `cargo test -p ps1-core --target <tu triple>` ejecuta las pruebas.
- `cargo run --release -p ps1-core --example boot --features trace ... --trace 50` imprime las
  últimas 50 instrucciones ejecutadas (muy lento: usa el camino exacto de la CPU).
- La salida del puerto serie (SIO1) que escriba la consola se muestra al final de `boot`.
- Más detalle: [README](../README.md) y [ROADMAP](../ROADMAP.md).
