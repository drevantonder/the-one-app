import { createRoot } from "react-dom/client"
import { GoogleSettings } from "../google/settings.jsx"

createRoot(document.getElementById("root")).render(<GoogleSettings title="Google Calendar" blurb="Sign in to Google, and your agents can read your calendars and, when you ask, add and change events." />)
