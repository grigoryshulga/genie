// The global foundation (brand tokens, resets, shell-wide rules) has to be the first stylesheet in
// the app's CSS, so it stays ahead of the sheets each screen imports for its own markup.
import "./styles.css";
import { QueryClientProvider } from "@tanstack/react-query";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { RouterProvider } from "react-router";
import { router } from "./App.tsx";
import { queryClient } from "@/shared/api";
import { ToastProvider } from "@/shared/ui";
import { applyAppearancePreferences, readAppearancePreferences } from "@/shared/lib";

applyAppearancePreferences(readAppearancePreferences());

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <ToastProvider>
        <RouterProvider router={router} />
      </ToastProvider>
    </QueryClientProvider>
  </StrictMode>,
);
