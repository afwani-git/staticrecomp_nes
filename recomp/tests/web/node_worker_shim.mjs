// Menjalankan web/worker.js di worker_threads Node dengan API ala browser Worker.
import { parentPort, workerData } from 'node:worker_threads';
import { pathToFileURL } from 'node:url';

globalThis.self = globalThis;
globalThis.postMessage = (message, transfer) => parentPort.postMessage(message, transfer);
parentPort.on('message', (data) => globalThis.onmessage({ data }));
await import(pathToFileURL(workerData.workerPath).href);
