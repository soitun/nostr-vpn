import type { QrMatrix, UiState } from './types';

const rawApiBase = import.meta.env.VITE_NVPN_API_BASE ?? '';
const apiBase = rawApiBase.replace(/\/+$/, '');

async function responseError(response: Response): Promise<string> {
  const contentType = response.headers.get('content-type') ?? '';
  if (contentType.includes('application/json')) {
    const body = (await response.json().catch(() => null)) as { error?: string } | null;
    if (body?.error) {
      return body.error;
    }
  }

  const text = await response.text().catch(() => '');
  return text.trim() || `${response.status} ${response.statusText}`;
}

async function postJson<T>(path: string, payload?: unknown): Promise<T> {
  const headers: Record<string, string> = {};
  const init: RequestInit = {
    method: 'POST',
    headers,
  };

  if (payload !== undefined) {
    headers['content-type'] = 'application/json';
    init.body = JSON.stringify(payload);
  }

  const response = await fetch(`${apiBase}${path}`, init);
  if (!response.ok) {
    throw new Error(await responseError(response));
  }
  return (await response.json()) as T;
}

export function tick(): Promise<UiState> {
  return postJson<UiState>('/api/tick');
}

export function runAction(endpoint: string, payload?: unknown): Promise<UiState> {
  return postJson<UiState>(endpoint, payload);
}

export function qrMatrix(text: string): Promise<QrMatrix> {
  return postJson<QrMatrix>('/api/qr_matrix', { text });
}
