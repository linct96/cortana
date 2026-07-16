if (import.meta.env.DEV) {
  import('react-grab');
}

import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { RouterProvider } from '@tanstack/react-router';
import { BackendGate } from './components/backend-gate';
import { Toaster } from './components/ui/sonner';
import { TooltipProvider } from './components/ui/tooltip';
import { router } from './router';
import './styles.css';

const rootElement = document.getElementById('root')!;
let root = (globalThis as any).__reactRoot;
if (!root) {
  root = createRoot(rootElement);
  (globalThis as any).__reactRoot = root;
}

root.render(
  <StrictMode>
    <TooltipProvider>
      <BackendGate>
        <RouterProvider router={router} />
      </BackendGate>
      <Toaster position="top-center" duration={2000} richColors />
    </TooltipProvider>
  </StrictMode>,
);
