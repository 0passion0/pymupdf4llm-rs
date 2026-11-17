import { useState } from 'react';
import { TopBar, type WorkspaceTab } from './components/TopBar';
import { WorkbenchView } from './components/WorkbenchView';
import { AsyncPipelineView } from './components/AsyncPipelineView';
import { RustCrateView } from './components/RustCrateView';
import { CompatibilityView } from './components/CompatibilityView';

export default function App() {
  const [activeTab, setActiveTab] = useState<WorkspaceTab>('workbench');

  return (
    <div className="min-h-screen bg-slate-50 text-slate-900">
      <TopBar activeTab={activeTab} onSelectTab={setActiveTab} />

      <main>
        {activeTab === 'workbench' && <WorkbenchView />}
        {activeTab === 'async' && <AsyncPipelineView />}
        {activeTab === 'crate' && <RustCrateView />}
        {activeTab === 'compatibility' && <CompatibilityView />}
      </main>
    </div>
  );
}
