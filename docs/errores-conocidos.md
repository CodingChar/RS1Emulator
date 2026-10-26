# Errores conocidos

## 1. «ACCESS ERROR» al guardar en Resident Evil (tarjeta 1, web)

**Síntoma.** En la pantalla «MEMORY CARD 1» de Resident Evil (USA), al intentar guardar la partida aparece `ACCESS ERROR`. Ocurre en el emulador web (`http://localhost:8080`, navegador Chrome/Edge).

**Captura.** `C:\Users\Pollito\AppData\Local\Temp\claude\C--Users-Pollito-Desktop-developments-RS1Emu\29e30781-238e-4de5-a7e7-e5770378966a\images\1.png`

**Estado en la captura.** La barra de estado dice «Tarjeta 2 formateada.», así que la tarjeta 2 se formateó, pero el error aparece con la tarjeta 1 seleccionada en el juego. Falta comprobar si la tarjeta 1 llegó a formatearse o si la tarjeta 2 es la que el juego usa.

**Dónde mirar.**
- Web: `web/app.js` (sección «Memory cards», `CARD_COUNT = 2`), `web/worker.js` (`persistCards`, `cardCommand`, líneas 286–320) y `web/storage.js` (IndexedDB, clave `card${port}`).
- Núcleo: `crates/ps1-core/src/controllers/memory_card.rs` (formato, cabecera, sectores y checksums) y `crates/ps1-core/src/controllers/sio.rs` (protocolo de la tarjeta).
- `crates/ps1-core/src/system.rs`: `insert_memory_card`, `insert_formatted_memory_card` y `memory_card_data`.

**Hipótesis a comprobar (sin verificar).**
1. El juego escribe en la tarjeta 1, pero el frontend solo guarda la tarjeta cuando `memory_card_dirty` está activo, así que puede que la escritura no llegue a IndexedDB.
2. La tarjeta 1 no está formateada: el juego lee un sistema de archivos vacío y muestra el error al no encontrarlo.
3. El protocolo de escritura falla con un checksum o un sector inválido, y el juego lo presenta como `ACCESS ERROR`. Los tests de `memory_card.rs` cubren checksums incorrectos, así que la lógica básica parece correcta; falta comprobar la ruta completa con el juego real.
4. El estado de la tarjeta se guarda o se carga mal entre el worker y el hilo principal al cambiar de ranura.

**Cómo reproducir.**
1. Abrir el emulador web, cargar la BIOS y el disco de Resident Evil.
2. Entrar en la pantalla de tarjetas del juego (MEMORY CARD 1) y guardar.
3. Observar si aparece `ACCESS ERROR` y anotar la barra de estado.

**Pendiente de comprobar.** Si la tarjeta 1 funciona tras formatear desde la interfaz web («Formatear» en el menú de tarjetas). Esto decide si el fallo es de formato o de escritura.

**Estado.** Abierto. No se ha reproducido con el arnés nativo todavía.
