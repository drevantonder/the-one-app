import { createRoot } from "react-dom/client"
import { GoogleSettings } from "../google/settings.jsx"

createRoot(document.getElementById("root")).render(<GoogleSettings title="Google Docs" blurb="Sign in to Google, and your agents can read your documents and, when you ask, write in them." />)
