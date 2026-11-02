// AudioWorklet: membaca sampel dari ring buffer SharedArrayBuffer yang diisi worker emulasi.
// Layout: Int32Array [0] posisi baca, [1] posisi tulis; lalu Float32Array data (mulai byte 8).
class NesAudio extends AudioWorkletProcessor {
  constructor(options) {
    super();
    const sab = options.processorOptions.sab;
    this.idx = new Int32Array(sab, 0, 2);
    this.data = new Float32Array(sab, 8);
  }

  process(_inputs, outputs) {
    const out = outputs[0];
    const ch0 = out[0];
    const n = this.data.length;
    let r = Atomics.load(this.idx, 0);
    const w = Atomics.load(this.idx, 1);
    for (let i = 0; i < ch0.length; i++) {
      if (r !== w) {
        ch0[i] = this.data[r];
        r = (r + 1) % n;
      } else {
        ch0[i] = 0; // kehabisan data: diam, jangan mengulang sampel lama
      }
    }
    Atomics.store(this.idx, 0, r);
    for (let c = 1; c < out.length; c++) out[c].set(ch0);
    return true;
  }
}

registerProcessor('nes-audio', NesAudio);
