/** Keeps the most recently used entries within an entry count and a total size budget. */
export class LruCache<V> {
  private readonly entries = new Map<string, { value: V; size: number }>();
  private totalSize = 0;
  private readonly maxEntries: number;
  private readonly maxSize: number;

  constructor(maxEntries: number, maxSize: number) {
    this.maxEntries = maxEntries;
    this.maxSize = maxSize;
  }

  get(key: string): V | undefined {
    const entry = this.entries.get(key);
    if (!entry) return undefined;
    this.entries.delete(key);
    this.entries.set(key, entry);
    return entry.value;
  }

  set(key: string, value: V, size: number) {
    this.delete(key);
    if (size > this.maxSize) return;
    this.entries.set(key, { value, size });
    this.totalSize += size;
    for (const oldest of this.entries.keys()) {
      if (this.entries.size <= this.maxEntries && this.totalSize <= this.maxSize) break;
      this.delete(oldest);
    }
  }

  delete(key: string) {
    const entry = this.entries.get(key);
    if (!entry) return;
    this.entries.delete(key);
    this.totalSize -= entry.size;
  }
}
