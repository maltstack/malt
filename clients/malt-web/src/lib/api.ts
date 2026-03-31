const API_BASE = 'http://127.0.0.1:7700';

export interface Session {
  id: number;
  name: string | null;
  pane_count: number;
  isolation: string;
  state: string;
}

export interface StyledSpan {
  t: string;
  fg: [number, number, number];
  bg: [number, number, number];
  b: boolean;
}

export async function listSessions(): Promise<Session[]> {
  const res = await fetch(`${API_BASE}/sessions`);
  const json = await res.json();
  return json.data;
}

export async function createSession(name?: string): Promise<Session> {
  const res = await fetch(`${API_BASE}/sessions`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ name: name || null }),
  });
  const json = await res.json();
  return json.data;
}

export async function getOutput(sessionId: number): Promise<StyledSpan[][]> {
  const res = await fetch(`${API_BASE}/sessions/${sessionId}/output`);
  const json = await res.json();
  return json.data?.rows || [];
}

export async function sendInput(sessionId: number, input: string): Promise<void> {
  await fetch(`${API_BASE}/sessions/${sessionId}/send`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ input }),
  });
}

export async function execCommand(sessionId: number, command: string): Promise<string> {
  const res = await fetch(`${API_BASE}/sessions/${sessionId}/exec`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ command }),
  });
  const json = await res.json();
  return json.data?.output || '';
}

export async function destroySession(sessionId: number): Promise<void> {
  await fetch(`${API_BASE}/sessions/${sessionId}`, { method: 'DELETE' });
}
