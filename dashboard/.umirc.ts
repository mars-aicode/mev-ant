import { defineConfig } from 'umi';

export default defineConfig({
  routes: [
    {
      path: '/',
      component: '@/layouts/BaseLayout',
      routes: [
        { path: '/', redirect: '/dashboard' },
        { path: '/dashboard', component: 'Dashboard', name: 'Dashboard' },
        { path: '/sandwiches', component: 'Sandwiches', name: 'Sandwiches' },
        { path: '/sandwiches/:id', component: 'SandwichDetail' },
        { path: '/attackers', component: 'Attackers', name: 'Attackers' },
        { path: '/replay', component: 'Replay', name: 'Replay' },
      ],
    },
  ],
  npmClient: 'npm',
  title: 'mev-ant',
  esbuildMinifyIIFE: true,
});
