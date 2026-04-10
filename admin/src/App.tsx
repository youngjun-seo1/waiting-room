import { useState } from 'react';
import { getConnection } from './api';
import { Login } from './pages/Login';
import { Dashboard } from './pages/Dashboard';
import { SchedulesPage } from './pages/SchedulesPage';
import { Layout } from './components/Layout';

type Page = 'dashboard' | 'schedules';

function App() {
  const [loggedIn, setLoggedIn] = useState(() => {
    const conn = getConnection();
    return !!conn.apiKey;
  });
  const [page, setPage] = useState<Page>('dashboard');

  if (!loggedIn) {
    return <Login onLogin={() => setLoggedIn(true)} />;
  }

  return (
    <Layout page={page} onNavigate={setPage} onLogout={() => setLoggedIn(false)}>
      {page === 'dashboard' && <Dashboard />}
      {page === 'schedules' && <SchedulesPage />}
    </Layout>
  );
}

export default App;
