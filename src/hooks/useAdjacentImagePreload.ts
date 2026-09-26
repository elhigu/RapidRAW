import { useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ImageFile, Invokes, LibraryDisplayMode } from '../components/ui/AppProperties';
import { useEditorStore } from '../store/useEditorStore';
import { useProcessStore } from '../store/useProcessStore';
import { useSettingsStore } from '../store/useSettingsStore';
import { useUIStore } from '../store/useUIStore';
import { hasCullingPreview, loadCullingPreview } from '../utils/cullingPreview';

// Wait for the user to pause on an image, so quickly held arrow keys don't queue work.
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
  const isLibraryView = useUIStore((s) => s.activeView === 'library');
  const isCullMode = useSettingsStore((s) => s.appSettings?.libraryDisplayMode === LibraryDisplayMode.Cull);
  const isCulling = isLibraryView && isCullMode;
  const isPreviewShown = useProcessStore((s) => (selectedPath ? Boolean(s.previews[selectedPath]) : false));
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

  // The culling view shows a full-resolution render, so prepare the neighbours' renders too,
  // once the current image's own preview is on screen.
  useEffect(() => {
    if (!enabled || !isCulling || !selectedPath || !isPreviewShown) return;
    let cancelled = false;
    const timer = setTimeout(async () => {
      for (const path of neighbours(listRef.current, selectedPath)) {
        if (cancelled) return;
        const thumbKey = useProcessStore.getState().thumbnails[path] || '';
        if (hasCullingPreview(path, thumbKey)) continue;
        await loadCullingPreview(path, thumbKey).catch((err) => console.warn('Preloading preview failed:', err));
      }
    }, PRELOAD_DELAY_MS);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [enabled, isCulling, selectedPath, isPreviewShown]);
}
