import { ProTable } from '@ant-design/pro-components';
import { Card, Row, Col, Input, InputNumber, Button, Space } from 'antd';
import { useEffect, useState } from 'react';
import { history, useSearchParams } from 'umi';
import { fetchSandwiches } from '@/services/api';
import { formatProfit } from '@/utils/format';

interface Filters {
  attacker?: string;
  block_from?: number;
  block_to?: number;
}

export default function SandwichesPage() {
  const [searchParams] = useSearchParams();
  const [filters, setFilters] = useState<Filters>({});

  useEffect(() => {
    const a = searchParams.get('attacker');
    if (a) {
      setFilters({ attacker: a });
    }
  }, []);

  const columns = [
    { title: 'Block', dataIndex: 'block_number', key: 'block' },
    { title: 'Attacker', dataIndex: 'attacker', key: 'attacker',
      render: (_: any, record: any) => {
        const addr = record.attacker;
        const s = typeof addr === 'string' ? addr : (addr?.toString?.() ?? String(addr));
        return <code style={{ fontSize: 11 }}>{s}</code>;
      } },
    { title: 'Profit', dataIndex: 'profit', key: 'profit',
      render: (_: any, r: any) => formatProfit(r.profit) },
    { title: 'V', dataIndex: 'victim_count', key: 'victims' },
    { title: 'Scanned', dataIndex: 'scanned_at', key: 'scanned_at' },
  ];

  const applyFilters = (f: Filters) => setFilters({ ...f });

  return (
    <>
      <Card style={{ marginBottom: 16 }}>
        <Row gutter={12} align="middle">
          <Col>
            <Input
              placeholder="Attacker address"
              value={filters.attacker || ''}
              onChange={e => setFilters(prev => ({ ...prev, attacker: e.target.value || undefined }))}
              style={{ width: 360 }}
              allowClear
            />
          </Col>
          <Col>
            <InputNumber
              placeholder="From block"
              value={filters.block_from}
              onChange={v => setFilters(prev => ({ ...prev, block_from: v || undefined }))}
              style={{ width: 140 }}
            />
          </Col>
          <Col>
            <InputNumber
              placeholder="To block"
              value={filters.block_to}
              onChange={v => setFilters(prev => ({ ...prev, block_to: v || undefined }))}
              style={{ width: 140 }}
            />
          </Col>
          <Col>
            <Button type="primary" onClick={() => applyFilters({ ...filters })}>
              Search
            </Button>
          </Col>
          <Col>
            <Button onClick={() => setFilters({})}>Reset</Button>
          </Col>
        </Row>
      </Card>

      <ProTable
        columns={columns}
        params={filters}
        request={async (params) => {
          const { pageSize, current } = params;
          return fetchSandwiches({
            page: current || 1,
            pageSize: pageSize || 20,
            ...filters,
          }).then(data => ({ data: data.sandwiches, total: data.total, success: true }));
        }}
        rowKey="id"
        search={false}
        pagination={{ pageSize: 20 }}
        onRow={(record) => ({
          style: { cursor: 'pointer' },
          onClick: () => history.push(`/sandwiches/${record.id}`),
        })}
      />
    </>
  );
}
