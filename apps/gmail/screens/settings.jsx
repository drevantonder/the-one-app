import { createRoot } from "react-dom/client"
import { GoogleSettings } from "../google/settings.jsx"

createRoot(document.getElementById("root")).render(<GoogleSettings title="Gmail" blurb="Sign in to Gmail, and your agents can search, read and draft your email." />)
