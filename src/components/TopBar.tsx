import { Download } from 'lucide-react';

export type WorkspaceTab = 'workbench' | 'async' | 'crate' | 'compatibility';

interface TopBarProps {
  activeTab: WorkspaceTab;
  onSelectTab: (tab: WorkspaceTab) => void;
}

const NAV_ITEMS: Array<{ id: WorkspaceTab; label: string }> = [
  { id: 'workbench', label: 'Workbench' },
  { id: 'async', label: 'Async Pipeline' },
  { id: 'crate', label: 'Rust Crate' },
  { id: 'compatibility', label: 'Compatibility' },
];

export function TopBar({ activeTab, onSelectTab }: TopBarProps) {
  return (
    <header className="border-b border-slate-200 bg-white md:sticky md:top-0 md:z-30">
      <div className="mx-auto flex max-w-[1440px] items-center justify-between gap-8 px-4 py-3 sm:px-6">
        <a
          href="#top"
          onClick={(e) => {
            e.preventDefault();
            onSelectTab('workbench');
          }}
          className="text-lg font-bold tracking-tight text-slate-900 whitespace-nowrap shrink-0"
        >
          pymupdf4llm-rs
        </a>

        <nav aria-label="Primary sections" className="hidden items-center gap-7 md:flex">
          {NAV_ITEMS.map((item) => {
            const active = activeTab === item.id;
            return (
              <button
                key={item.id}
                type="button"
                onClick={() => onSelectTab(item.id)}
                aria-current={active ? 'page' : undefined}
                className={`relative py-2 text-sm font-medium transition-colors whitespace-nowrap shrink-0 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-blue-600 ${
                  active ? 'text-blue-600' : 'text-slate-600 hover:text-slate-900'
                }`}
              >
                {item.label}
                {active && (
                  <span
                    aria-hidden="true"
                    className="absolute inset-x-0 -bottom-[13px] h-0.5 bg-blue-600"
                  />
                )}
              </button>
            );
          })}
        </nav>

        <div className="shrink-0">
          <a
            href="/api/crate/download"
            download="pymupdf4llm-rs-0.4.0.tar.gz"
            className="inline-flex min-h-10 items-center gap-2 rounded-lg bg-blue-600 px-4 py-2 text-xs font-semibold text-white transition-colors hover:bg-blue-700 whitespace-nowrap shrink-0 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-blue-600"
          >
            <Download className="h-3.5 w-3.5" aria-hidden="true" />
            <span>Download Crate</span>
          </a>
        </div>
      </div>

      <nav
        aria-label="Mobile sections"
        className="flex items-center gap-5 overflow-x-auto border-t border-slate-100 px-4 py-1.5 md:hidden"
      >
        {NAV_ITEMS.map((item) => {
          const active = activeTab === item.id;
          return (
            <button
              key={item.id}
              type="button"
              onClick={() => onSelectTab(item.id)}
              aria-current={active ? 'page' : undefined}
              className={`min-h-9 py-1 text-xs font-medium whitespace-nowrap shrink-0 ${
                active ? 'text-blue-600 underline underline-offset-4' : 'text-slate-600'
              }`}
            >
              {item.label}
            </button>
          );
        })}
      </nav>
    </header>
  );
}
