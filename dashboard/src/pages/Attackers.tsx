import { ProTable } from '@ant-design/pro-components';
import { history } from 'umi';
import { fetchAttackers } from '@/services/api';

export default function AttackersPage() {
  const columns = [
    { title: 'Address', dataIndex: 'address', key: 'address',
      render: (t: string) => <a onClick={() => history.push(`/sandwiches?attacker=${t}`)}><code style={{ fontSize: 11 }}>{t}</code></a> },
    { title: 'Sandwiches', dataIndex: 'sandwich_count', key: 'count', width: 100 },
    { title: 'First Seen', dataIndex: 'first_seen', key: 'first', width: 120 },
    { title: 'Last Seen', dataIndex: 'last_seen', key: 'last', width: 120 },
  ];

  return (
    <>
      <ProTable
        columns={columns}
        request={async () => {
          const data = await fetchAttackers();
          return { data, total: data.length, success: true };
        }}
        rowKey="address"
        search={false}
        pagination={{ pageSize: 20 }}
      />
    </>
  );
}
