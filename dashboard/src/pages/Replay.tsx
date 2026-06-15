import { Card, InputNumber, Button, Space, message, Alert, Statistic, Row, Col } from 'antd';
import { ReloadOutlined } from '@ant-design/icons';
import { useEffect, useState } from 'react';
import { fetchState, requestReplay } from '../services/api';

export default function ReplayPage() {
  const [fromBlock, setFromBlock] = useState<number | undefined>();
  const [loading, setLoading] = useState(false);
  const [pendingReplay, setPendingReplay] = useState<number>(0);
  const [nextBlock, setNextBlock] = useState<number>(0);

  const refresh = async () => {
    try {
      const state = await fetchState();
      setPendingReplay(state.pending_replay_from ?? 0);
      setNextBlock(state.next_block ?? 0);
    } catch {
      // ignore — keep last known values
    }
  };

  useEffect(() => {
    refresh();
    const id = setInterval(refresh, 2000);
    return () => clearInterval(id);
  }, []);

  const handleReplay = async () => {
    if (!fromBlock || fromBlock <= 0) {
      message.warning('Enter a valid block number');
      return;
    }
    setLoading(true);
    try {
      const data = await requestReplay(fromBlock);
      if (data.status === 'queued') {
        message.success(`Replay queued from block ${data.from_block}. The scanner will perform it on its next iteration.`);
        refresh();
      } else {
        message.error(data.message || 'Replay failed');
      }
    } catch {
      message.error('Replay request failed');
    } finally {
      setLoading(false);
    }
  };

  const isPending = pendingReplay > 0;

  return (
    <Card title="Replay Blocks" style={{ maxWidth: 600 }}>
      <Alert
        type={isPending ? 'info' : 'warning'}
        showIcon
        message={
          isPending
            ? `Replay pending from block ${pendingReplay}. The scanner will pick it up on its next iteration.`
            : 'This will queue a replay: the scanner deletes all sandwich data from the given block onward and restarts scanning from that block on its next iteration.'
        }
        style={{ marginBottom: 16 }}
      />
      <Row gutter={16} style={{ marginBottom: 16 }}>
        <Col span={12}>
          <Statistic title="Next block" value={nextBlock} />
        </Col>
        <Col span={12}>
          <Statistic
            title="Pending replay from"
            value={isPending ? pendingReplay : '—'}
            valueStyle={isPending ? { color: '#1677ff' } : undefined}
          />
        </Col>
      </Row>
      <Space>
        <InputNumber
          min={1}
          placeholder="From block number"
          value={fromBlock}
          onChange={v => setFromBlock(v || undefined)}
          style={{ width: 200 }}
        />
        <Button
          type="primary"
          icon={<ReloadOutlined />}
          loading={loading}
          onClick={handleReplay}
        >
          {isPending ? 'Queue another replay' : 'Replay'}
        </Button>
      </Space>
    </Card>
  );
}
