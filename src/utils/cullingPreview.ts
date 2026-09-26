import { invoke } from '@tauri-apps/api/core';
import { Invokes } from '../components/ui/AppProperties';
import { useProcessStore } from '../store/useProcessStore';

type SidecarMetadata = { adjustments?: (Record<string, unknown> & { is_null?: boolean }) | null } | null;

const pending = new Map<string, Promise<string>>();
const pendingKey = (path: string, thumbKey: string) => `${path}\n${thumbKey}`;

async function renderPreview(path: string): Promise<string> {
  let bytes: Uint8Array;
  try {
    const metadata = await invoke<SidecarMetadata>(Invokes.LoadMetadata, { path });
    const adjustments = metadata?.adjustments && !metadata.adjustments.is_null ? metadata.adjustments : {};
    bytes = await invoke<Uint8Array>(Invokes.GeneratePreviewForPath, { path, jsAdjustments: adjustments });
  } catch (err) {
    console.error('Error loading culling preview with adjustments:', err);
    bytes = await invoke<Uint8Array>(Invokes.GeneratePreviewForPath, { path, jsAdjustments: {} });
  }
  return URL.createObjectURL(new Blob([new Uint8Array(bytes)], { type: 'image/jpeg' }));
}

export function hasCullingPreview(path: string, thumbKey: string): boolean {
  return useProcessStore.getState().previews[path]?.thumbKey === thumbKey;
}

export function isCullingPreviewPending(path: string, thumbKey: string): boolean {
  return pending.has(pendingKey(path, thumbKey));
}

// Renders and stores the full-resolution culling preview. Concurrent requests for the same
// image, such as a preload and the culling view itself, share one render.
export function loadCullingPreview(path: string, thumbKey: string): Promise<string> {
  const key = pendingKey(path, thumbKey);
  const existing = pending.get(key);
  if (existing) return existing;

  const request = renderPreview(path)
    .then((url) => {
      useProcessStore.getState().setPreview(path, url, thumbKey);
      return url;
    })
    .finally(() => pending.delete(key));
  pending.set(key, request);
  return request;
}
