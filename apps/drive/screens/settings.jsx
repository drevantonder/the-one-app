import { createRoot } from "react-dom/client"
import { GoogleSettings } from "../google/settings.jsx"

createRoot(document.getElementById("root")).render(<GoogleSettings title="Google Drive" blurb="Sign in, and your apps can save copies to your Drive." />)
