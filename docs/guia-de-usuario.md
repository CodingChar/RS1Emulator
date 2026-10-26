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
herramientas. Pausa el juego mientras está abierto y no deja pasar ningún botón al juego.
Se maneja con las flechas y Intro, con el mando (cruceta, ✕ para elegir, ○ para volver) o con
el ratón. Ofrece:

- reanudar y reiniciar,
- guardar y cargar estado en tres ranuras,
- cambiar el disco con el juego en marcha (los juegos de varios discos lo piden),
- velocidad, modo analógico, volumen y silencio,
- relación de aspecto (4:3 o rellenar) y filtro de imagen (píxeles nítidos o suavizado),
- pantalla completa y ajustes.

En pantalla completa, **Esc** abre el menú rápido (mantenlo pulsado para salir de la pantalla
completa; si tu navegador no lo permite, usa **F1**). **Alt + Intro** alterna la pantalla completa.

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
| Va a saltos | Baja la velocidad objetivo o cierra otras pestañas pesadas. |
| Una tarjeta se perdió | El navegador pudo borrar sus datos; importa una copia exportada. |

## Para quien desarrolla

- `cargo test -p ps1-core --target <tu triple>` ejecuta las pruebas.
- `cargo run --release -p ps1-core --example boot --features trace ... --trace 50` imprime las
  últimas 50 instrucciones ejecutadas (muy lento: usa el camino exacto de la CPU).
- La salida del puerto serie (SIO1) que escriba la consola se muestra al final de `boot`.
- Más detalle: [README](../README.md) y [ROADMAP](../ROADMAP.md).
