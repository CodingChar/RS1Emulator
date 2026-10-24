// Persistent storage in IndexedDB for the BIOS, memory cards and save states.
// Falls back to memory (lost on reload) where IndexedDB is unavailable, for
// example in some private-browsing modes.

const DB_NAME = 'rs1emu';
const STORE = 'files';

const fallback = new Map();
let dbPromise = null;

function open() {
    if (!dbPromise) {
        dbPromise = new Promise((resolve, reject) => {
            if (typeof indexedDB === 'undefined') {
                reject(new Error('IndexedDB is not available'));
                return;
            }
            const request = indexedDB.open(DB_NAME, 1);
            request.onupgradeneeded = () => request.result.createObjectStore(STORE);
            request.onsuccess = () => resolve(request.result);
            request.onerror = () => reject(request.error);
        }).catch((error) => {
            console.warn('Persistent storage unavailable, using memory only:', error);
            return null;
        });
    }
    return dbPromise;
}

function request(db, mode, run) {
    return new Promise((resolve, reject) => {
        const transaction = db.transaction(STORE, mode);
        const result = run(transaction.objectStore(STORE));
        transaction.oncomplete = () => resolve(result.result);
        transaction.onerror = () => reject(transaction.error);
        transaction.onabort = () => reject(transaction.error);
    });
}

/** Whether data survives a page reload. */
export async function isPersistent() {
    return (await open()) !== null;
}

/** Read a stored Uint8Array, or undefined. */
export async function get(key) {
    const db = await open();
    if (!db) return fallback.get(key);
    return request(db, 'readonly', (store) => store.get(key));
}

export async function put(key, bytes) {
    const db = await open();
    if (!db) {
        fallback.set(key, bytes);
        return;
    }
    await request(db, 'readwrite', (store) => store.put(bytes, key));
}

export async function remove(key) {
    const db = await open();
    if (!db) {
        fallback.delete(key);
        return;
    }
    await request(db, 'readwrite', (store) => store.delete(key));
}
