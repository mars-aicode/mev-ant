import { Card, Button, Space, Table, Badge } from 'antd';
import { PauseCircleOutlined, PlayCircleOutlined } from '@ant-design/icons';
import { useEffect, useState } from 'react';
import { history, useNavigate } from 'umi';
import { fetchStats, fetchState, fetchSandwiches, fetchAttackers, pauseScan, resumeScan } from '@/services/api';
import { formatProfit } from '@/utils/format';

export default function DashboardPage() {
  const [stats, setStats] = useState<any>({});
  const [state, setState] = useState<any>({ enabled: true, next_block: 0 });
  const [sandwiches, setSandwiches] = useState<any[]>([]);
  const [attackers, setAttackers] = useState<any[]>([]);
  const navigate = useNavigate();

  const loadData = async () => {
    const [s, st, sand, att] = await Promise.all([
      fetchStats(), fetchState(),
      fetchSandwiches({ page: 1, pageSize: 5 }),
      fetchAttackers(),
    ]);
    setStats(s);
    setState(st);
    setSandwiches(sand.sandwiches || []);
    setAttackers((att || []).slice(0, 10));
  };

  useEffect(() => { loadData(); const t = setInterval(loadData, 5000); return () => clearInterval(t); }, []);

  const handlePause = async () => { await pauseScan(); loadData(); };
  const handleResume = async () => { await resumeScan(); loadData(); };

  const lag = Math.max(0, (stats.chain_head || 0) - (stats.current_block || 0));
  const lagColor = !state.enabled ? 'red' : lag < 3 ? 'green' : lag < 20 ? 'orange' : 'red';

  const sandwichColumns = [
    { title: 'Block', dataIndex: 'block_number', key: 'block' },
    { title: 'Attacker', dataIndex: 'attacker', key: 'attacker',
      render: (_: any, record: any) => {
        const addr = record.attacker;
        const s = typeof addr === 'string' ? addr : (addr?.toString?.() ?? String(addr));
        return <a onClick={() => navigate(`/sandwiches?attacker=${s}`)}><code style={{ fontSize: 11 }}>{s}</code></a>;
      } },
    { title: 'Profit', dataIndex: 'profit', key: 'profit',
      render: (_: any, r: any) => formatProfit(r.profit) },
    { title: 'V', dataIndex: 'victim_count', key: 'victims' },
  ];

  const attackerColumns = [
    { title: 'Address', dataIndex: 'address', key: 'addr',
      render: (t: string) => {
        const addr = typeof t === 'string' ? t : String(t);
        return <a onClick={() => navigate(`/sandwiches?attacker=${addr}`)}><code style={{ fontSize: 11 }}>{addr}</code></a>;
      } },
    { title: 'Sandwiches', dataIndex: 'sandwich_count', key: 'count' },
    { title: 'First', dataIndex: 'first_seen', key: 'first' },
    { title: 'Last', dataIndex: 'last_seen', key: 'last' },
  ];

  return (
    <>
      {/* Status bar */}
      <div style={{ display: 'flex', alignItems: 'center', padding: '8px 16px',
        background: '#fafafa', borderRadius: 6, marginBottom: 16, gap: 12, flexWrap: 'wrap' }}>
        <Badge color={lagColor} />
        <span style={{ fontFamily: 'monospace', fontSize: 13 }}>
          Scanner: {stats.current_block?.toLocaleString() || '-'} / {stats.chain_head?.toLocaleString() || '-'}
          &nbsp;|&nbsp; Lag: <span style={{ color: lagColor, fontWeight: 600 }}>{lag}</span> blocks
        </span>
        <span style={{ flex: 1 }} />
        {state.enabled ? (
          <Button icon={<PauseCircleOutlined />} danger size="small" onClick={handlePause}>Pause</Button>
        ) : (
          <Button icon={<PlayCircleOutlined />} type="primary" size="small" onClick={handleResume}>Resume</Button>
        )}
        <span style={{ fontSize: 11, color: '#999' }}>Sandwiches: {stats.total_sandwiches || 0}</span>
        <span style={{ fontSize: 11, color: '#999' }}>Attackers: {stats.distinct_attackers || 0}</span>
      </div>

      <Card
        title={
          <Space>
            Recent Sandwiches ({stats.total_sandwiches || 0})
            <a onClick={() => history.push('/sandwiches')} style={{ fontSize: 13, fontWeight: 'normal' }}>View All</a>
          </Space>
        }
        bodyStyle={{ padding: 0 }}
        style={{ marginBottom: 16 }}
      >
        <Table
          dataSource={sandwiches}
          columns={sandwichColumns}
          rowKey="id"
          pagination={false}
          size="small"
          onRow={(record) => ({
            style: { cursor: 'pointer' },
            onClick: () => navigate(`/sandwiches/${record.id}`),
          })}
        />
      </Card>

      <Card
        title={
          <Space>
            Top Attackers ({stats.distinct_attackers || 0})
            <a onClick={() => history.push('/attackers')} style={{ fontSize: 13, fontWeight: 'normal' }}>View All</a>
          </Space>
        }
        bodyStyle={{ padding: 0 }}
      >
        <Table
          dataSource={attackers}
          columns={attackerColumns}
          rowKey="address"
          pagination={false}
          size="small"
        />
      </Card>
    </>
  );
}
