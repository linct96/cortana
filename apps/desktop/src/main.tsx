if (import.meta.env.DEV) {
  import('react-grab');
}

import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { RouterProvider } from '@tanstack/react-router';
import { Toaster } from './components/ui/sonner';
import { TooltipProvider } from './components/ui/tooltip';
import { router } from './router';
import './styles.css';

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <TooltipProvider>
      <RouterProvider router={router} />
      <Toaster position="top-center" duration={2000} richColors />
    </TooltipProvider>
  </StrictMode>,
);
