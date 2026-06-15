// API client for mev-ant backend
// Uses relative URLs — works when dashboard is served from same origin as API
const BASE = '';

export async function fetchStats() {
  const res = await fetch(`${BASE}/api/stats`);
  return res.json();
}

export async function fetchSandwiches(params?: {
  page?: number;
  pageSize?: number;
  attacker?: string;
  block_from?: number;
  block_to?: number;
}) {
  const q = new URLSearchParams();
  if (params?.page) q.set('page', String(params.page));
  if (params?.pageSize) q.set('page_size', String(params.pageSize));
  if (params?.attacker) q.set('attacker', params.attacker);
  if (params?.block_from) q.set('block_from', String(params.block_from));
  if (params?.block_to) q.set('block_to', String(params.block_to));
  const res = await fetch(`${BASE}/api/sandwiches?${q}`);
  return res.json();
}

export async function fetchDetail(id: number) {
  const res = await fetch(`${BASE}/api/sandwich?id=${id}`);
  return res.json();
}

export async function fetchAttackers() {
  const res = await fetch(`${BASE}/api/attackers`);
  return res.json();
}

export async function fetchState() {
  const res = await fetch(`${BASE}/api/state`);
  return res.json();
}

export async function requestReplay(fromBlock: number) {
  const res = await fetch(`${BASE}/api/replay`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ from_block: fromBlock }),
  });
  return res.json();
}

export async function pauseScan() {
  await fetch(`${BASE}/api/state/pause`, { method: 'POST' });
}

export async function resumeScan() {
  await fetch(`${BASE}/api/state/resume`, { method: 'POST' });
}
