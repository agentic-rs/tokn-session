import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { consumeLoginToken } from "./lib/login";

const initial_token = consumeLoginToken();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App initial_token={initial_token} />
  </React.StrictMode>,
);
