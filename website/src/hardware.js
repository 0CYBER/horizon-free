// The hardware the Benchmarks page can estimate for, from specifications only (our own table, no benchmark
// database): each GPU's FP32 shader throughput in TFLOPS and its memory bandwidth in GB/s as its maker states them,
// and each CPU's single-thread score = its top boost clock (GHz) x our rough per-generation factor (instructions per
// clock, Zen 4 = 1). The factors are round numbers, so estimates are ranges, and the page says so.
// `match` finds a reported name (the GPU and CPU strings in benchmark reports) in this table.

export const GPUS = [
  { name: "NVIDIA GeForce RTX 4090", tflops: 82.6, gbps: 1008, match: /RTX 4090/i },
  { name: "NVIDIA GeForce RTX 4080", tflops: 48.7, gbps: 717, match: /RTX 4080(?! SUPER)/i },
  { name: "NVIDIA GeForce RTX 4070 Ti", tflops: 40.1, gbps: 504, match: /RTX 4070 Ti/i },
  { name: "NVIDIA GeForce RTX 4070", tflops: 29.1, gbps: 504, match: /RTX 4070(?! Ti)/i },
  { name: "NVIDIA GeForce RTX 4060 Ti", tflops: 22.1, gbps: 288, match: /RTX 4060 Ti/i },
  { name: "NVIDIA GeForce RTX 4060", tflops: 15.1, gbps: 272, match: /RTX 4060(?! Ti)/i },
  { name: "NVIDIA GeForce RTX 3080", tflops: 29.8, gbps: 760, match: /RTX 3080(?! Ti)/i },
  { name: "NVIDIA GeForce RTX 3070", tflops: 20.3, gbps: 448, match: /RTX 3070(?! Ti)/i },
  { name: "NVIDIA GeForce RTX 3060", tflops: 12.7, gbps: 360, match: /RTX 3060(?! Ti)/i },
  { name: "NVIDIA GeForce RTX 3050", tflops: 9.1, gbps: 224, match: /RTX 3050/i },
  { name: "NVIDIA GeForce RTX 2060", tflops: 6.5, gbps: 336, match: /RTX 2060(?! SUPER)/i },
  { name: "NVIDIA GeForce GTX 1660 SUPER", tflops: 5.0, gbps: 336, match: /GTX 1660 SUPER/i },
  { name: "NVIDIA GeForce GTX 1060 6GB", tflops: 4.4, gbps: 192, match: /GTX 1060/i },
  { name: "NVIDIA GeForce GTX 1050 Ti", tflops: 2.1, gbps: 112, match: /GTX 1050 Ti/i },
  { name: "AMD Radeon RX 7900 XTX", tflops: 61.4, gbps: 960, match: /RX 7900 XTX/i },
  { name: "AMD Radeon RX 7800 XT", tflops: 37.3, gbps: 624, match: /RX 7800 XT/i },
  { name: "AMD Radeon RX 7600", tflops: 21.8, gbps: 288, match: /RX 7600(?! XT)/i },
  { name: "AMD Radeon RX 6700 XT", tflops: 13.2, gbps: 384, match: /RX 6700 XT/i },
  { name: "AMD Radeon RX 6600", tflops: 8.9, gbps: 224, match: /RX 6600(?! XT)/i },
  { name: "AMD Radeon RX 580", tflops: 6.2, gbps: 256, match: /RX 580/i },
  { name: "AMD Radeon 780M (integrated)", tflops: 8.6, gbps: 89, match: /780M/i },
  { name: "Steam Deck (AMD Van Gogh)", tflops: 1.6, gbps: 88, match: /VanGogh|Van Gogh|Custom GPU 0405/i },
  { name: "Intel Arc A770", tflops: 19.7, gbps: 560, match: /Arc.*A770/i },
  { name: "Intel Iris Xe Graphics (96 EU)", tflops: 2.1, gbps: 68, match: /Iris\(?R?\)? Xe/i },
  { name: "Intel HD Graphics 620", tflops: 0.4, gbps: 34, match: /\bU?HD Graphics 620\b/i },
  { name: "Apple M1", tflops: 2.6, gbps: 68, match: /^Apple M1$/i },
  { name: "Apple M2", tflops: 3.6, gbps: 100, match: /^Apple M2$/i },
  { name: "Apple M3", tflops: 4.1, gbps: 100, match: /^Apple M3$/i },
  { name: "Apple M4", tflops: 4.3, gbps: 120, match: /^Apple M4$/i },
  { name: "Apple M1 Pro (16-core GPU)", tflops: 5.2, gbps: 200, match: /^Apple M1 Pro$/i },
  { name: "Apple M4 Pro (20-core GPU)", tflops: 8.5, gbps: 273, match: /^Apple M4 Pro$/i },
  { name: "Apple M4 Max (40-core GPU)", tflops: 17.0, gbps: 546, match: /^Apple M4 Max$/i },
];

// ghz: top single-core boost clock; ipc: our per-generation factor (Zen 4 = 1).
export const CPUS = [
  { name: "Intel Core i7-7500U", ghz: 3.5, ipc: 0.62, match: /i7-7500U/i },
  { name: "Intel Core i5-7200U", ghz: 3.1, ipc: 0.62, match: /i5-7200U/i },
  { name: "Intel Core i5-8250U", ghz: 3.4, ipc: 0.62, match: /i5-8250U/i },
  { name: "Intel Core i5-9400F", ghz: 4.1, ipc: 0.62, match: /i5-9400/i },
  { name: "Intel Core i7-9700K", ghz: 4.9, ipc: 0.62, match: /i7-9700K/i },
  { name: "Intel Core i5-10400", ghz: 4.3, ipc: 0.62, match: /i5-10400/i },
  { name: "Intel Core i5-1135G7", ghz: 4.2, ipc: 0.78, match: /i5-1135G7/i },
  { name: "Intel Core i5-12400", ghz: 4.4, ipc: 0.95, match: /i5-12400/i },
  { name: "Intel Core i5-13600K", ghz: 5.1, ipc: 0.95, match: /i5-13600K/i },
  { name: "Intel Core i7-13700K / KF", ghz: 5.4, ipc: 0.95, match: /i7-13700K/i },
  { name: "Intel Core i9-14900K", ghz: 6.0, ipc: 0.95, match: /i9-14900K/i },
  { name: "AMD Ryzen 5 3600", ghz: 4.2, ipc: 0.72, match: /Ryzen 5 3600/i },
  { name: "AMD Ryzen 5 5600X", ghz: 4.6, ipc: 0.88, match: /Ryzen 5 5600X?\b/i },
  { name: "AMD Ryzen 7 5800X3D", ghz: 4.5, ipc: 0.88, match: /5800X3D/i },
  { name: "AMD Ryzen 5 7600X", ghz: 5.3, ipc: 1.0, match: /Ryzen 5 7600/i },
  { name: "AMD Ryzen 7 7800X3D", ghz: 5.0, ipc: 1.0, match: /7800X3D/i },
  { name: "AMD Ryzen 9 7950X", ghz: 5.7, ipc: 1.0, match: /Ryzen 9 7950X(?!3D)/i },
  { name: "AMD Ryzen 7 9700X", ghz: 5.5, ipc: 1.12, match: /Ryzen 7 9700X/i },
  { name: "AMD Ryzen 7 7840U / HS", ghz: 5.1, ipc: 1.0, match: /7840(U|HS)/i },
  { name: "Steam Deck (AMD Zen 2, 4 cores)", ghz: 3.5, ipc: 0.72, match: /AMD Custom APU 0405|Van ?Gogh/i },
  { name: "Apple M1", ghz: 3.2, ipc: 1.25, match: /^Apple M1$/i },
  { name: "Apple M2", ghz: 3.5, ipc: 1.28, match: /^Apple M2$/i },
  { name: "Apple M3", ghz: 4.05, ipc: 1.38, match: /^Apple M3$/i },
  { name: "Apple M4", ghz: 4.4, ipc: 1.5, match: /^Apple M4$/i },
  { name: "Apple M4 Pro", ghz: 4.5, ipc: 1.5, match: /^Apple M4 Pro$/i },
];

export const singleThread = (cpu) => cpu.ghz * cpu.ipc;
export const findGpu = (name) => GPUS.find((g) => g.name === name || g.match.test(name || "")) || null;
export const findCpu = (name) => CPUS.find((c) => c.name === name || c.match.test(name || "")) || null;
