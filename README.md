# RAM Map Viewer

A simple egui application to explore memory maps.

Memory map format can be seen below.

```
  [pa00_0008_0000 - pa00_0020_0000) |   2 MiB | (Drop) C   RW PXN | Init_Thread
  [pa00_0020_0000 - pa00_0020_7260) |  29 KiB | (Free) C   RW PXN | RAM
  [pa00_0040_0000 - pa00_0040_3800) |  14 KiB | (Used) C   RO PX  | Nucleus code
  [pa00_4000_0000 - pa00_4000_0100) | 256 B   | (Used) Dev RW PXN | local_intc
```

Start physical address
End physical address (exclusive)
Size
(Free), (Used), (Drop) flag - (Drop) is freeable mem, that can be reclaimed.
C for Cacheable DRAM, Dev for Device/MMIO memory
Read, Write, Prog Execute Never
and a Name

Provide file path with the memory map as cmd line argument.
