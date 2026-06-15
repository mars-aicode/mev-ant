import { ProLayout } from '@ant-design/pro-components';
import { Outlet, useLocation, useNavigate } from 'umi';
import { DashboardOutlined, TableOutlined, UserOutlined, ReloadOutlined } from '@ant-design/icons';

export default function BaseLayout() {
  const location = useLocation();
  const navigate = useNavigate();

  return (
    <ProLayout
      title="mev-ant"
      logo={false}
      location={location}
      route={{
        routes: [
          { path: '/dashboard', name: 'Dashboard', icon: <DashboardOutlined /> },
          { path: '/sandwiches', name: 'Sandwiches', icon: <TableOutlined /> },
          { path: '/attackers', name: 'Attackers', icon: <UserOutlined /> },
          { path: '/replay', name: 'Replay', icon: <ReloadOutlined /> },
        ],
      }}
      menuItemRender={(item, dom) => (
        <a onClick={() => navigate(item.path!)}>{dom}</a>
      )}
    >
      <Outlet />
    </ProLayout>
  );
}
