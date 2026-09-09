import { createRoot } from "react-dom/client";
import { App } from "./App";
import "./styles.css";

const root = document.getElementById("root");

if (root === null) {
  throw new Error("OpenMat root element is missing");
}

const dynamicAppModules = (import.meta.env.VITE_OPENMAT_APP_MODULES ?? "")
  .split(";")
  .map((value) => value.trim())
  .filter((value) => value.length > 0);

createRoot(root).render(<App dynamicAppModules={dynamicAppModules} />);
