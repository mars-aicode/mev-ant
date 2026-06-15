import { useParams, history } from 'umi';
import { Button, Spin, Empty } from 'antd';
import { ArrowLeftOutlined } from '@ant-design/icons';
import { useEffect, useState } from 'react';
import { fetchDetail } from '@/services/api';
import BundleDetailView from '@/components/BundleDetailView';

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

  return (
    <>
      <Button icon={<ArrowLeftOutlined />} onClick={() => history.push('/sandwiches')} style={{ marginBottom: 16 }}>
        Back to Sandwiches
      </Button>
      <BundleDetailView data={data} />
    </>
  );
}
