// MIDI音高与键位分离，布局只消费原生编译结果，不重复推导调性。
const STEPS = [0, 2, 4, 5, 7, 9, 11];
export const DEFAULT_KEYS = Array.from({ length: 21 }, (_, i) => ({ pitch: 48 + Math.floor(i / 7) * 12 + STEPS[i % 7], label: String(i % 7 + 1) }));
export const JUDGMENTS = {
  coarse: { label: "粗判", window: 120, rank: 1 },
  strict: { label: "严判", window: 70, rank: 2 },
  ultra: { label: "超严判", window: 40, rank: 3 },
};
export const DIFFICULTIES = { green: "绿谱", red: "红谱", expert: "高难谱" };
export function pitchName(midi) { return `${["C", "C♯", "D", "E♭", "E", "F", "F♯", "G", "A♭", "A", "B♭", "B"][midi % 12]}${Math.floor(midi / 12) - 1}`; }
export function recordKey(song) { return `${song.id}@${song.version}|lyre21|${song.arrangement}`; }
