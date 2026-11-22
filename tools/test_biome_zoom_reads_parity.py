#!/usr/bin/env python3
"""`LevelReader.getBiome(BlockPos)`'s zoom for spawn variants and /execute if biome (#14980)."""
import sys
import test_command_exec_parity as driver
from biome_zoom_reads_corpus import biome_zoom_reads

if __name__ == '__main__':
    if '--work-dir' not in sys.argv:
        sys.argv += ['--work-dir', str(driver.lab.LOCAL / 'biome-zoom-reads-parity')]
    driver.corpus_module.groups = lambda: [
        ('zoomed biome reads', [*driver.corpus_module.group_setup(), *biome_zoom_reads()])]
    driver.main()
