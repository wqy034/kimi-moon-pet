// 真实录音采样器；只按音频时钟预约声音，不用JS定时器决定音符落点。
export class PianoAudio {
  constructor() { this.context = null; this.master = null; this.samples = []; this.voices = new Set(); this.volume = .25; this.muted = false; this.loading = null; this.assistEnabled = true; }
  async enable() {
    this.context ||= new AudioContext({ latencyHint: "interactive" });
    // 保留在用户点击的同步调用栈内，先解锁再等待文件读取。
    const resume = this.context.resume();
    if (!this.master) {
      this.master = this.context.createGain(); this.master.gain.value = this.muted ? 0 : this.volume;
      // 自由弹奏可能多键叠加，温和压缩避免大音量时削波。
      this.limiter = this.context.createDynamicsCompressor();
      this.limiter.threshold.value = -9; this.limiter.knee.value = 6; this.limiter.ratio.value = 12;
      this.limiter.attack.value = .003; this.limiter.release.value = .1;
      this.master.connect(this.limiter); this.limiter.connect(this.context.destination);
      this.assistGain = this.context.createGain(); this.assistGain.gain.value = this.assistEnabled ? 1 : 0;
      this.assistGain.connect(this.master);
    }
    await resume;
    if (this.samples.length) return;
    this.loading ||= this.load().catch(error => { this.loading = null; this.samples = []; throw error; });
    await this.loading;
  }
  async load() {
    const response = await fetch("audio/piano/manifest.json");
    if (!response.ok) throw Error("钢琴音源目录读取失败");
    const manifest = await response.json();
    const samples = [];
    for (const item of manifest.samples) {
      const file = await fetch(`audio/piano/${item.file}`);
      if (!file.ok) throw Error(`采样缺失：${item.file}`);
      const decoded = await this.context.decodeAudioData(await file.arrayBuffer());
      const length = Math.min(decoded.length, Math.floor(decoded.sampleRate * 3));
      const buffer = this.context.createBuffer(decoded.numberOfChannels, length, decoded.sampleRate);
      for (let channel = 0; channel < decoded.numberOfChannels; channel++) buffer.copyToChannel(decoded.getChannelData(channel).subarray(0, length), channel);
      samples.push({ midi: item.midi, buffer });
    }
    if (!samples.length) throw Error("钢琴音源为空");
    this.samples = samples;
  }
  fromPerformance(time) {
    const ctx = this.context;
    if (!ctx) return 0;
    const stamp = ctx.getOutputTimestamp?.();
    if (stamp?.contextTime > 0 && stamp.performanceTime > 0 && Math.abs(performance.now() - stamp.performanceTime) < 1000) {
      return stamp.contextTime + (time - stamp.performanceTime) / 1000;
    }
    return ctx.currentTime + (time - performance.now()) / 1000;
  }
  now() { return this.fromPerformance(performance.now()); }
  setVolume(value, muted = this.muted) {
    this.volume = Math.max(0, Math.min(.8, value)); this.muted = muted;
    this.master?.gain.setTargetAtTime(muted ? 0 : this.volume, this.context.currentTime, .015);
  }
  setAssist(enabled) {
    this.assistEnabled = enabled;
    this.assistGain?.gain.setTargetAtTime(enabled ? 1 : 0, this.context.currentTime, .01);
    if (!enabled) for (const voice of this.voices) if (voice.assist) this.release(voice);
  }
  playPitch(midi, when = this.context.currentTime, duration = 1.2, assist = false, velocity = 96) {
    if (!this.samples.length || this.context.state !== "running") return null;
    if (!Number.isInteger(midi) || midi < 0 || midi > 127) return null;
    const sample = this.samples.reduce((best, item) => Math.abs(item.midi - midi) < Math.abs(best.midi - midi) ? item : best);
    const source = this.context.createBufferSource(), gain = this.context.createGain();
    source.buffer = sample.buffer; source.playbackRate.value = 2 ** ((midi - sample.midi) / 12);
    source.connect(gain); gain.connect(assist ? this.assistGain : this.master);
    when = Math.max(when, this.context.currentTime); duration = Math.min(2.4, Math.max(.06, duration));
    const level = Math.min(1, .8 * (Math.max(1, Math.min(127, velocity)) / 96) ** 1.6);
    gain.gain.setValueAtTime(0, when); gain.gain.linearRampToValueAtTime(level, when + .006);
    gain.gain.setValueAtTime(level, when + duration); gain.gain.exponentialRampToValueAtTime(.0001, when + duration + .13);
    source.start(when); source.stop(when + duration + .15);
    const voice = { source, gain, assist, released: false };
    source.onended = () => { this.voices.delete(voice); source.disconnect(); gain.disconnect(); };
    if ([...this.voices].filter(v => !v.released).length >= 64) this.release([...this.voices].find(v => !v.released));
    this.voices.add(voice); return voice;
  }
  release(voice) {
    if (!voice || voice.released) return;
    voice.released = true;
    const now = this.context.currentTime;
    voice.gain.gain.cancelScheduledValues(now);
    voice.gain.gain.setTargetAtTime(.0001, now, .025);
    try { voice.source.stop(now + .12); } catch { /* 已结束的录音无需再次释放 */ }
  }
  stopAll() { for (const voice of this.voices) this.release(voice); }
}
