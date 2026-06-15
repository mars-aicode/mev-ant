import { useParams, history } from 'umi';
import { Card, Descriptions, Table, Button, Spin, Empty, Row, Col, Tag } from 'antd';
import { ArrowLeftOutlined, LinkOutlined } from '@ant-design/icons';
import { useEffect, useState } from 'react';
import { fetchDetail } from '@/services/api';
import { formatProfit, formatAmount, formatEth, tokenSymbol } from '@/utils/format';

interface Transfer {
  token: string;
  from: string;
  to: string;
  amount: string;
}

function fmtAddr(addr: string) {
  if (!addr) return 'N/A';
  return addr.startsWith('0x') ? addr : `0x${addr}`;
}

const ROLE_COLORS: Record<string, string> = {
  A: 'red', F: 'orange', E: 'purple',
  I: 'blue', B: 'geekblue', T: 'green',
  P: 'magenta', C: 'gold',
};

function etherscanLink(type: 'tx' | 'address', id: string) {
  if (!id) return '#';
  return `https://etherscan.io/${type}/${id}`;
}

/** Build tag list for an address from the role map */
function addrTags(addr: string, roleMap: Record<string, { chars: string[]; colors: string[] }>) {
  const key = (addr || '').toLowerCase();
  const entry = roleMap[key];
  if (!entry) return null;
  return entry.chars.map((c, i) => (
    <Tag key={c} color={entry.colors[i]} style={{ marginLeft: 2, fontSize: 10, lineHeight: '14px', padding: '0 3px' }}>{c}</Tag>
  ));
}

function makeTransferColumns(roleMap: Record<string, { chars: string[]; colors: string[] }>) {
  const addrRender = (t: string) => (
    <span style={{ whiteSpace: 'nowrap' }}>
      <code style={{ fontSize: 11 }}>{t}</code>
      {addrTags(t, roleMap)}
    </span>
  );
  return [
    { title: 'Token', dataIndex: 'token', key: 'token',
      render: (t: string) => {
        const s = tokenSymbol(t);
        const label = s || (t.startsWith('0x') ? `0x${t.slice(2, 6)}...` : `${t.slice(0, 6)}...`);
        return <span style={{ whiteSpace: 'nowrap' }}>
          <a href={etherscanLink('address', t)} target="_blank" rel="noopener noreferrer" style={{ fontSize: 11, fontFamily: 'monospace' }}>{label}</a>
          {addrTags(t, roleMap)}
        </span>;
      } },
    { title: 'From', dataIndex: 'from', key: 'from', render: addrRender, ellipsis: true },
    { title: 'To', dataIndex: 'to', key: 'to', render: addrRender, ellipsis: true },
    { title: 'Amount', dataIndex: 'amount', key: 'amount',
      render: (_: any, record: any) => <span style={{ whiteSpace: 'nowrap' }}>{formatAmount(record.token, record.amount)}</span> },
  ];
}

export default function SandwichDetailPage() {
  const { id } = useParams<{ id: string }>();
  const [data, setData] = useState<any>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');

  useEffect(() => {
    const idNum = parseInt(id || '', 10);
    if (isNaN(idNum) || idNum <= 0) { setError('Invalid sandwich id'); setLoading(false); return; }
    fetchDetail(idNum)
      .then(setData)
      .catch(() => setError('Failed to load sandwich detail'))
      .finally(() => setLoading(false));
  }, [id]);

  if (loading) return <div style={{ textAlign: 'center', padding: 80 }}><Spin size="large" /></div>;
  if (error) return <div style={{ textAlign: 'center', padding: 80 }}><Empty description={error} /></div>;
  if (!data) return <div style={{ textAlign: 'center', padding: 80 }}><Empty description="No data" /></div>;

  let frontTransfers: Transfer[] = [];
  let victimTransfers: Transfer[] = [];
  let backTransfers: Transfer[] = [];
  const profitDisplay = formatProfit(data.profit_json || '[]');

  // Multi-role map: each address can have multiple roles
  const roleMap: Record<string, { chars: string[]; colors: string[] }> = {};
  const addRole = (addr: string, ch: string) => {
    if (!addr) return;
    const key = (addr.startsWith('0x') ? addr : `0x${addr}`).toLowerCase();
    const color = ROLE_COLORS[ch] || 'default';
    if (roleMap[key]) {
      roleMap[key].chars.push(ch);
      roleMap[key].colors.push(color);
    } else {
      roleMap[key] = { chars: [ch], colors: [color] };
    }
  };
  addRole(data.attacker, 'A');
  addRole(data.funder, 'F');
  addRole(data.executor, 'E');
  addRole(data.initiator, 'I');
  addRole(data.back_initiator, 'B');
  addRole(data.target, 'T');
  addRole(data.coinbase, 'C');
  addRole(data.attacked_pool, 'P');
  const transferColumns = makeTransferColumns(roleMap);
  try { frontTransfers = JSON.parse(data.front_transfers || '[]'); } catch {}
  try { victimTransfers = JSON.parse(data.victim_transfers || '[]'); } catch {}
  try { backTransfers = JSON.parse(data.back_transfers || '[]'); } catch {}

  return (
    <>
      <Button icon={<ArrowLeftOutlined />} onClick={() => history.push('/sandwiches')} style={{ marginBottom: 16 }}>
        Back to Sandwiches
      </Button>

      <Card title={`Sandwich Bundle — Block ${data.block_number}`} style={{ marginBottom: 16 }}>
        <Descriptions column={2} size="small" bordered>
          <Descriptions.Item label="Front TX Index">{data.front_tx_index}</Descriptions.Item>
          <Descriptions.Item label="Back TX Index">{data.back_tx_index}</Descriptions.Item>
          <Descriptions.Item label="Victim Count">{data.victim_count}</Descriptions.Item>
          {[{addr: data.attacked_pool, label: 'Attacked Pool'},
            {addr: data.attacker,      label: 'Attacker'},
            {addr: data.funder,        label: 'Funder'},
            {addr: data.executor,      label: 'Executor'},
            {addr: data.initiator,     label: 'Initiator'},
            {addr: data.back_initiator, label: 'Back Initiator'},
            {addr: data.target,        label: 'Target'},
            {addr: data.coinbase,      label: 'Coinbase'},
          ].map(r => (
            <Descriptions.Item key={r.label} label={r.label}>
              <a href={etherscanLink('address', fmtAddr(r.addr))} target="_blank" rel="noopener noreferrer">
                <code style={{ fontSize: 11 }}>{fmtAddr(r.addr)} <LinkOutlined /></code>
              </a>
            </Descriptions.Item>
          ))}
          <Descriptions.Item label="Profit" span={2}>{profitDisplay}</Descriptions.Item>
          <Descriptions.Item label="Gas Cost">{data.gas_cost_wei != null ? formatEth(data.gas_cost_wei) : '-'}</Descriptions.Item>
          <Descriptions.Item label="Coinbase Bribe">{data.coinbase_bribe != null ? formatEth(data.coinbase_bribe) : '-'}</Descriptions.Item>
          <Descriptions.Item label="Expense">{data.expense_wei != null ? formatEth(data.expense_wei) : '-'}</Descriptions.Item>
          <Descriptions.Item label="Scanned" span={2}>{data.created_at}</Descriptions.Item>
        </Descriptions>
      </Card>

      {/* Transfer graphs */}
      <Card title="Frontrun Transfers" style={{ marginBottom: 16, overflow: 'hidden' }}>
        {frontTransfers.length > 0
          ? <Table dataSource={frontTransfers} columns={transferColumns} rowKey={(_, i) => String(i)} pagination={false} size="small" scroll={{ x: 'max-content' }} />
          : <Empty description="No frontrun transfers" />}
      </Card>

      <Card title="Backrun Transfers" style={{ marginBottom: 16, overflow: 'hidden' }}>
        {backTransfers.length > 0
          ? <Table dataSource={backTransfers} columns={transferColumns} rowKey={(_, i) => String(i)} pagination={false} size="small" scroll={{ x: 'max-content' }} />
          : <Empty description="No backrun transfers" />}
      </Card>

      <Card title="Victim Transfers" style={{ marginBottom: 16, overflow: 'hidden' }}>
        {victimTransfers.length > 0
          ? <Table dataSource={victimTransfers} columns={transferColumns} rowKey={(_, i) => String(i)} pagination={false} size="small" scroll={{ x: 'max-content' }} />
          : <Empty description="No victim transfers" />}
      </Card>

      {/* TX Cards */}
      <Row gutter={16} style={{ marginBottom: 16 }}>
        <Col span={12}>
          <Card title="Frontrun TX">
            <Descriptions column={1} size="small" bordered>
              <Descriptions.Item label="Hash">
                {data.front_tx_hash ? (
                  <a href={etherscanLink('tx', data.front_tx_hash)} target="_blank" rel="noopener noreferrer">
                    <code style={{ fontSize: 11 }}>{data.front_tx_hash} <LinkOutlined /></code>
                  </a>
                ) : 'N/A'}
              </Descriptions.Item>
            </Descriptions>
          </Card>
        </Col>
        <Col span={12}>
          <Card title="Backrun TX">
            <Descriptions column={1} size="small" bordered>
              <Descriptions.Item label="Hash">
                {data.back_tx_hash ? (
                  <a href={etherscanLink('tx', data.back_tx_hash)} target="_blank" rel="noopener noreferrer">
                    <code style={{ fontSize: 11 }}>{data.back_tx_hash} <LinkOutlined /></code>
                  </a>
                ) : 'N/A'}
              </Descriptions.Item>
            </Descriptions>
          </Card>
        </Col>
      </Row>

      {/* Victim TXs */}
      {(() => {
        let hashes: string[] = [];
        try { hashes = JSON.parse(data.victim_tx_hashes || '[]'); } catch {}
        if (!hashes.length) return null;
        return (
          <Card title={`Victim TXs (${hashes.length})`} style={{ overflow: 'hidden' }}>
            <Table
              dataSource={hashes.map((h, i) => ({ key: i, hash: h }))}
              columns={[
                { title: '#', dataIndex: 'key', key: 'key', width: 60 },
                { title: 'Hash', dataIndex: 'hash', key: 'hash',
                  render: (h: string) => (
                    <a href={etherscanLink('tx', h)} target="_blank" rel="noopener noreferrer">
                      <code style={{ fontSize: 11 }}>{h} <LinkOutlined /></code>
                    </a>
                  ) },
              ]}
              pagination={false}
              size="small"
              scroll={{ x: 'max-content' }}
            />
          </Card>
        );
      })()}
    </>
  );
}
