import { useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ImageFile, Invokes } from '../components/ui/AppProperties';
import { useEditorStore } from '../store/useEditorStore';
import { useSettingsStore } from '../store/useSettingsStore';

// Wait for the user to pause on an image, so quickly held arrow keys don't queue decodes.
const PRELOAD_DELAY_MS = 300;
// Matches PRELOAD_MIN_CACHE_SIZE in the backend: the current image and both neighbours need room.
const PRELOAD_MIN_CACHE_SIZE = 4;

function neighbours(list: ImageFile[], path: string): string[] {
  const index = list.findIndex((image) => image.path === path);
  if (index === -1 || list.length < 2) return [];
  const next = list[(index + 1) % list.length].path;
  const previous = list[(index - 1 + list.length) % list.length].path;
  return [...new Set([next, previous])].filter((p) => p !== path);
}

export function useAdjacentImagePreload(sortedImageList: ImageFile[]) {
  const enabled = useSettingsStore((s) => (s.appSettings?.imageCacheSize ?? 5) >= PRELOAD_MIN_CACHE_SIZE);
  const selectedPath = useEditorStore((s) => s.selectedImage?.path);
  const isReady = useEditorStore((s) => s.selectedImage?.isReady ?? false);
  const listRef = useRef(sortedImageList);

  useEffect(() => {
    listRef.current = sortedImageList;
  }, [sortedImageList]);

  useEffect(() => {
    if (!enabled || !selectedPath || !isReady) return;
    const timer = setTimeout(() => {
      const paths = neighbours(listRef.current, selectedPath);
      if (paths.length > 0) {
        invoke(Invokes.PreloadImages, { paths }).catch((err) => console.warn('Preloading images failed:', err));
      }
    }, PRELOAD_DELAY_MS);
    return () => clearTimeout(timer);
  }, [enabled, selectedPath, isReady]);
}
