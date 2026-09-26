import { useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ImageFile, Invokes } from '../components/ui/AppProperties';
import { useEditorStore } from '../store/useEditorStore';
import { useSettingsStore } from '../store/useSettingsStore';

// Wait for the user to pause on an image, so quickly held arrow keys don't queue decodes.
const PRELOAD_DELAY_MS = 300;

export function useAdjacentImagePreload(sortedImageList: ImageFile[]) {
  const enabled = useSettingsStore((s) => s.appSettings?.preloadAdjacentImages ?? false);
  const selectedPath = useEditorStore((s) => s.selectedImage?.path);
  const isReady = useEditorStore((s) => s.selectedImage?.isReady ?? false);
  const listRef = useRef(sortedImageList);

  useEffect(() => {
    listRef.current = sortedImageList;
  }, [sortedImageList]);

  useEffect(() => {
    if (!enabled || !selectedPath || !isReady) return;

    const timer = setTimeout(() => {
      const list = listRef.current;
      const index = list.findIndex((image) => image.path === selectedPath);
      if (index === -1 || list.length < 2) return;

      const next = list[(index + 1) % list.length].path;
      const previous = list[(index - 1 + list.length) % list.length].path;
      const paths = [...new Set([next, previous])].filter((path) => path !== selectedPath);
      invoke(Invokes.PreloadImages, { paths }).catch((err) => console.warn('Preloading images failed:', err));
    }, PRELOAD_DELAY_MS);

    return () => clearTimeout(timer);
  }, [enabled, selectedPath, isReady]);
}
