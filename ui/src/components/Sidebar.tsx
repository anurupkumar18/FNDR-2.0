export type SidebarScreen = "search" | "vault" | "settings" | "health";

interface SidebarItemDefinition {
  id: SidebarScreen;
  label: string;
  enabled: boolean;
}

const SIDEBAR_ITEMS: SidebarItemDefinition[] = [
  { id: "search", label: "Search", enabled: true },
  { id: "vault", label: "Memory Vault", enabled: false },
  { id: "settings", label: "Settings", enabled: false },
  { id: "health", label: "Pipeline Health", enabled: false },
];

export interface SidebarProps {
  active: SidebarScreen;
}

export function Sidebar({ active }: SidebarProps) {
  return (
    <nav className="sidebar" aria-label="FNDR">
      <p className="sidebar-brand">FNDR</p>
      <ul className="sidebar-list">
        {SIDEBAR_ITEMS.map((item) => (
          <li key={item.id}>
            <button
              type="button"
              className="sidebar-item"
              data-active={item.id === active}
              disabled={!item.enabled}
              aria-current={item.id === active ? "page" : undefined}
            >
              {item.label}
              {!item.enabled && <span className="sidebar-item-badge">Soon</span>}
            </button>
          </li>
        ))}
      </ul>
    </nav>
  );
}
