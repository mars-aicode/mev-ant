import { Card, InputNumber, Button, Space, Row, Col, List, Spin, Empty, message } from 'antd';
import { SearchOutlined } from '@ant-design/icons';
import { useState } from 'react';
import { checkBlock } from '@/services/api';
import BundleDetailView from '@/components/BundleDetailView';

interface Bundle {
  id: number;
  attacker: string;
  block_number: number;
  front_tx_index: number;
  back_tx_index: number;
}

function shortAddr(addr?: string) {
  if (!addr) return '-';
  const s = addr.startsWith('0x') ? addr : `0x${addr}`;
  return `${s.slice(0, 8)}...`;
}

export default function TestCheckPage() {
  const [blockNumber, setBlockNumber] = useState<number | undefined>();
  const [loading, setLoading] = useState(false);
  const [bundles, setBundles] = useState<Bundle[]>([]);
  const [selected, setSelected] = useState<any>(null);

  const handleCheck = async () => {
    if (!blockNumber || blockNumber <= 0) {
      message.warning('Enter a valid block number');
      return;
    }
    setLoading(true);
    setSelected(null);
    try {
      const data = await checkBlock(blockNumber);
      const list = (data.bundles || []) as Bundle[];
      setBundles(list);
      if (list.length > 0) {
        setSelected(list[0]);
      } else {
        message.info('No sandwiches detected in this block');
      }
    } catch (e: any) {
      message.error(e.message || 'Detect request failed');
    } finally {
      setLoading(false);
    }
  };

  return (
    <>
      <Card title="Test Check" style={{ marginBottom: 16 }}>
        <Space>
          <InputNumber
            min={1}
            placeholder="Block number"
            value={blockNumber}
            onChange={v => setBlockNumber(v || undefined)}
            style={{ width: 220 }}
          />
          <Button
            type="primary"
            icon={<SearchOutlined />}
            loading={loading}
            onClick={handleCheck}
          >
            Check
          </Button>
        </Space>
      </Card>

      {bundles.length > 0 && (
        <Row gutter={16}>
          <Col span={6}>
            <Card title={`Sandwiches (${bundles.length})`} bodyStyle={{ padding: 0 }}>
              <List
                size="small"
                dataSource={bundles}
                renderItem={(b, idx) => (
                  <List.Item
                    style={{
                      cursor: 'pointer',
                      padding: '8px 16px',
                      background: selected?.id === b.id ? '#e6f7ff' : undefined,
                    }}
                    onClick={() => setSelected(b)}
                  >
                    <code style={{ fontSize: 12 }}>
                      #{idx + 1} {shortAddr(b.attacker)}
                    </code>
                  </List.Item>
                )}
              />
            </Card>
          </Col>
          <Col span={18}>
            {selected ? (
              <BundleDetailView data={selected} />
            ) : (
              <Card>
                <Empty description="Select a sandwich from the list" />
              </Card>
            )}
          </Col>
        </Row>
      )}
    </>
  );
}
