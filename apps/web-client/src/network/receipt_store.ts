export const STORAGE_KEY_RECEIPTS = 'project_new_claimed_receipts';

export function loadClaimedReceipts(storage?: Storage): Set<string> {
  try {
    const s = storage ?? (typeof localStorage !== 'undefined' ? localStorage : null);
    const raw = s ? s.getItem(STORAGE_KEY_RECEIPTS) : null;
    return raw ? new Set(JSON.parse(raw)) : new Set();
  } catch {
    return new Set();
  }
}

export function saveClaimedReceipts(receipts: Set<string>, storage?: Storage): void {
  try {
    const s = storage ?? (typeof localStorage !== 'undefined' ? localStorage : null);
    if (s) {
      s.setItem(STORAGE_KEY_RECEIPTS, JSON.stringify(Array.from(receipts)));
    }
  } catch {
    // Ignore storage errors in restricted contexts
  }
}
