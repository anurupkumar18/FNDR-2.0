import { Sidebar } from "./components/Sidebar";
import { SearchScreen } from "./screens/SearchScreen";

export function App() {
  return (
    <div className="app-shell">
      <Sidebar active="search" />
      <main className="app-content">
        <SearchScreen />
      </main>
    </div>
  );
}
