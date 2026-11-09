// Settings: on each of the user's machines running Agent Devices, whether agents may borrow Chrome run straight on it,
// without a container. The service keeps the switch on each machine, with its default (on where the machine runs only its
// owner's calls).
import { useEffect, useState } from "react"
import { createRoot } from "react-dom/client"
import { Empty, Hint, List, ListSkeleton, LoadFailure, Problem, Switch } from "@one/ui"

const ChromeLogo = () => (
  <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 48 48" width="28" height="28" aria-hidden="true">
    <defs>
      <linearGradient id="a" x1="3.2173" y1="15" x2="44.7812" y2="15" gradientUnits="userSpaceOnUse"><stop offset="0" stopColor="#d93025" /><stop offset="1" stopColor="#ea4335" /></linearGradient>
      <linearGradient id="b" x1="20.7219" y1="47.6791" x2="41.5039" y2="11.6837" gradientUnits="userSpaceOnUse"><stop offset="0" stopColor="#fcc934" /><stop offset="1" stopColor="#fbbc04" /></linearGradient>
      <linearGradient id="c" x1="26.5981" y1="46.5015" x2="5.8161" y2="10.506" gradientUnits="userSpaceOnUse"><stop offset="0" stopColor="#1e8e3e" /><stop offset="1" stopColor="#34a853" /></linearGradient>
    </defs>
    <circle cx="24" cy="23.9947" r="12" fill="#fff" />
    <path d="M24,12H44.7812a23.9939,23.9939,0,0,0-41.5639.0029L13.6079,30l.0093-.0024A11.9852,11.9852,0,0,1,24,12Z" fill="url(#a)" />
    <circle cx="24" cy="24" r="9.5" fill="#1a73e8" />
    <path d="M34.3913,30.0029,24.0007,48A23.994,23.994,0,0,0,44.78,12.0031H23.9989l-.0025.0093A11.985,11.985,0,0,1,34.3913,30.0029Z" fill="url(#b)" />
    <path d="M13.6086,30.0031,3.218,12.006A23.994,23.994,0,0,0,24.0025,48L34.3931,30.0029l-.0067-.0068a11.9852,11.9852,0,0,1-20.7778.007Z" fill="url(#c)" />
  </svg>
)

function Machine({ d, set }) {
  return (
    <label className="flex cursor-pointer items-center gap-3 px-3 py-2.5 text-sm has-[button:disabled]:cursor-default">
      <span className="min-w-0 flex-1">
        <span className="block truncate font-medium">{d.on}</span>
        {d.can && <span className="block text-muted-foreground">{d.can}</span>}
      </span>
      <Switch checked={!!d.allowed} disabled={!!d.can} onCheckedChange={(allowed) => set(d.on, allowed)} />
    </label>
  )
}

function Settings() {
  const [devices, setDevices] = useState(null)
  const [problem, setProblem] = useState(null)
  const [error, setError] = useState("")

  async function load() {
    try {
      setDevices((await one.call("_chrome", {})).devices)
      setProblem(null)
    } catch (e) {
      setProblem(one.plain(e))
    }
  }
  async function set(on, allowed) {
    setError("")
    try {
      await one.call("_set_chrome", { on, allowed })
    } catch (e) {
      setError(one.plain(e).title)
    }
    load()
  }
  useEffect(() => {
    load()
  }, [])

  let body
  if (!devices) body = problem ? <LoadFailure problem={problem} retry={load} /> : <ListSkeleton />
  else if (!devices.length) body = <Empty>None of your devices runs Agent Devices.</Empty>
  else body = (
    <>
      <List>{devices.map((d) => <Machine key={d.on} d={d} set={set} />)}</List>
      {error && <Problem title={error} />}
    </>
  )
  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-1">
        <h2 className="flex items-center gap-2.5 text-xl font-semibold">
          <ChromeLogo />
          Google Chrome
        </h2>
        <Hint>Chrome isn’t kept apart from the device it runs on. Turn it on only for agents you trust.</Hint>
      </div>
      {body}
    </div>
  )
}

createRoot(document.getElementById("root")).render(<Settings />)
