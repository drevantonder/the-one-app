defmodule GoogleCalendar do
  @moduledoc """
  Google Calendar: the person's own Google calendars, through Calendar's API (v3), from any device. Signing in and
  the accounts are the `google-account` plugin's, the same for every Google plugin. (A Mac's own calendars are another app's.)
  """
  @behaviour OneCore.Plugin

  @api "https://www.googleapis.com/calendar/v3/"
  @day 86_400

  # It has no app, so it names and draws itself, in the Plugins app.
  @impl true
  def title, do: google().title

  @impl true
  def icon, do: "icon.png"

  @doc "What Google Calendar asks of Google, for the `google-account` plugin."
  def google do
    %{
      title: "Google Calendar",
      scopes: ["https://www.googleapis.com/auth/calendar"]
    }
  end

  @impl true
  def requires, do: %{"google-account" => "to reach your calendars"}

  # Its person's Google accounts are google-account's, which signs one in with what each Google plugin asks of Google.
  @impl true
  def hooks, do: ["google-account.scopes"]

  @impl true
  def hook("google-account.scopes", _about, _call), do: google().scopes

  @impl true
  def tools do
    string = &%{type: "string", description: &1}

    account =
      string.("Which account: its address, or a part of it only it has, as google-calendar_accounts lists them. The first if left out")

    with_account = fn properties, required ->
      Map.merge(%{type: "object", properties: Map.put(properties, :account, account)}, if(required, do: %{required: required}, else: %{}))
    end

    calendar =
      string.("The calendar's id, like someone@gmail.com or ...@group.calendar.google.com, from google-calendar_calendars. primary if left out")

    event = string.("The event's id, from google-calendar_events. One repeat's id, like abc123_20261103T000000Z, names just that repeat")

    send_updates = %{
      type: "string",
      enum: ["all", "externalOnly", "none"],
      description: "Who Google emails about it: all guests, externalOnly (guests outside your organisation), or none, the default"
    }

    time = fn what, more ->
      %{
        type: "object",
        description:
          ~s[#{what}, as Calendar's EventDateTime: {"dateTime": "2026-11-02T09:30:00+11:00"}, {"dateTime": "2026-11-02T09:30:00", "timeZone": "Australia/Sydney"} (with neither, the calendar's time zone), or {"date": "2026-11-02"} for a whole day#{more}],
        properties: %{
          dateTime: string.("An RFC 3339 date-time, like 2026-11-02T09:30:00+11:00"),
          date: string.("A whole day, like 2026-11-02"),
          timeZone: string.("An IANA time zone, like Australia/Sydney")
        }
      }
    end

    fields = %{
      calendarId: calendar,
      summary: string.("The event's title, like Dentist"),
      start: time.("When it starts", ""),
      end: time.("When it ends", ", the day after its last. An hour after start if left out, or the next day"),
      location: string.("Where, like an address or a room"),
      description: string.("Notes, as plain text or simple HTML"),
      attendees: %{
        type: "array",
        description: ~s|The guests, each {"email": ..., "optional": ...}, like [{"email": "sam@example.org"}]. Google tells them only with sendUpdates|,
        items: %{
          type: "object",
          required: ["email"],
          properties: %{email: string.("Their address"), optional: %{type: "boolean", description: "Whether they're optional"}}
        }
      },
      recurrence: %{
        type: "array",
        items: %{type: "string"},
        description: ~s|How it repeats, as RFC 5545 lines, like ["RRULE:FREQ=WEEKLY;BYDAY=MO;COUNT=10"]|
      },
      sendUpdates: send_updates
    }

    # An event as the tools answer it: Calendar's Event resource, with the fields they pass on.
    person = %{
      type: "object",
      properties: %{email: %{type: "string"}, displayName: %{type: "string"}},
      required: ["email"]
    }

    listed = %{
      type: "object",
      properties: %{
        id: string.("The event's id"),
        status: string.("confirmed, tentative or cancelled"),
        summary: %{type: ["string", "null"], description: "Its title"},
        start: %{time.("When it starts", "") | type: ["object", "null"]},
        end: %{time.("When it ends", ", the day after its last for a whole day") | type: ["object", "null"]},
        location: %{type: ["string", "null"]},
        recurringEventId: %{type: ["string", "null"], description: "The repeating event this is one repeat of"},
        attendees: %{
          type: "array",
          items:
            Map.merge(person, %{
              properties:
                Map.merge(person.properties, %{
                  responseStatus: string.("needsAction, declined, tentative or accepted"),
                  optional: %{type: "boolean"}
                })
            })
        },
        htmlLink: string.("Its link in Google Calendar")
      },
      required: ["id", "status", "summary", "start", "end", "location", "recurringEventId", "attendees", "htmlLink"]
    }

    whole = %{
      listed
      | properties:
          Map.merge(listed.properties, %{
            description: %{type: ["string", "null"], description: "Its notes"},
            recurrence: %{type: "array", items: %{type: "string"}, description: "How it repeats, as RFC 5545 lines"},
            organizer: %{person | type: ["object", "null"]},
            hangoutLink: %{type: ["string", "null"], description: "Its Meet link"}
          }),
        required: listed.required ++ ["description", "recurrence", "organizer", "hangoutLink"]
    }

    [
      %{
        name: "accounts",
        read_only: true,
        description:
          "The Google accounts Google Calendar can use, the default first, each with ok, and why when it's false (google-calendar_sign_in with that account fixes it). Every other tool's account takes one of these.",
        inputs: %{type: "object", properties: %{}},
        outputs: %{
          type: "object",
          properties: %{
            accounts: %{
              type: "array",
              description: "The default first",
              items: %{
                type: "object",
                properties: %{
                  account: string.("Its address"),
                  ok: %{type: "boolean", description: "Whether it can be used now"},
                  why: string.("Why not, when ok is false: sign_in with that account fixes it")
                },
                required: ["account", "ok"]
              }
            }
          },
          required: ["accounts"]
        }
      },
      %{
        name: "sign_in",
        description:
          "Starts signing a Google account in to Google Calendar, or letting it use one another Google plugin has: gives url, for the person to open in their browser, where Google signs them in and sends them back to finish.",
        inputs: %{type: "object", properties: %{account: string.("The account's address, to suggest it to Google: for one that needs signing in again, as google-calendar_accounts lists it")}},
        outputs: %{
          type: "object",
          properties: %{url: string.("Where the person signs in, in their browser"), then: string.("What happens next")},
          required: ["url", "then"]
        }
      },
      %{
        name: "calendars",
        read_only: true,
        description:
          "The account's calendars (Calendar's calendarList.list), the primary first: each one's id (what calendarId takes elsewhere), name, your access (owner, writer or reader) and time zone.",
        inputs: with_account.(%{}, nil),
        outputs: %{
          type: "object",
          properties: %{
            calendars: %{
              type: "array",
              description: "The primary first",
              items: %{
                type: "object",
                properties: %{
                  id: string.("What calendarId takes elsewhere"),
                  summary: string.("Its name, as the person sees it"),
                  primary: %{type: "boolean"},
                  accessRole: string.("Your access: owner, writer, reader or freeBusyReader"),
                  timeZone: string.("An IANA time zone"),
                  hidden: %{type: "boolean", description: "Whether it's hidden in Google Calendar's list"}
                },
                required: ["id", "summary", "primary", "accessRole", "timeZone", "hidden"]
              }
            }
          },
          required: ["calendars"]
        }
      },
      %{
        name: "events",
        read_only: true,
        description:
          "Events in a time range, to see what's on today or this week or find a meeting (Calendar's events.list, each repeat on its own, soonest first): each event's id, start and end (a dateTime, or a date for a whole day), summary, where and guests. The coming week of the primary calendar if left out. google-calendar_event gives one event's notes and Meet link too. Where there are more, nextPageToken is the next page's pageToken.",
        inputs:
          with_account.(
            %{
              calendarId: calendar,
              timeMin:
                string.("The start of the range, RFC 3339, like 2026-11-02T00:00:00+11:00 (without an offset, or a date like 2026-11-02, in the calendar's time zone). Now if left out"),
              timeMax: string.("The end of the range, the same way. A week after timeMin if left out"),
              q: string.("Words to find in events' summaries, places, notes and guests, like standup"),
              maxResults: %{type: "integer", description: "At most this many. 50 if left out, and at most 250"},
              pageToken: string.("The next page: the nextPageToken the last events gave, with the same calendarId, timeMin and timeMax")
            },
            nil
          ),
        outputs: %{
          type: "object",
          properties: %{
            calendar: string.("The calendar's name"),
            timeZone: string.("The calendar's time zone"),
            timeMin: string.("The range's start, RFC 3339"),
            timeMax: string.("The range's end, RFC 3339"),
            events: %{type: "array", items: listed, description: "Soonest first, each repeat on its own"},
            nextPageToken: %{type: ["string", "null"], description: "The next page's pageToken, if there are more"}
          },
          required: ["calendar", "timeZone", "timeMin", "timeMax", "events", "nextPageToken"]
        }
      },
      %{
        name: "event",
        read_only: true,
        description:
          "One event (Calendar's events.get): its summary, when, how it repeats, where, organizer, guests and their answers, its Meet link, its link in Google Calendar and its notes.",
        inputs: with_account.(%{eventId: event, calendarId: calendar}, ["eventId"]),
        outputs: whole
      },
      %{
        name: "create_event",
        description:
          "Adds an event to a calendar (Calendar's events.insert). This changes the person's calendar: do it only when they asked. Guests are told only with sendUpdates. Gives the event, with its id and link.",
        inputs: with_account.(fields, ["summary", "start"]),
        outputs: listed
      },
      %{
        name: "update_event",
        description:
          "Changes an event (Calendar's events.patch): only what's given changes, and attendees and recurrence, given, replace the whole list. A repeating event's id changes every repeat, and one repeat's id just that one. This changes the person's calendar: do it only when they asked. Guests are told only with sendUpdates.",
        inputs: with_account.(Map.put(fields, :eventId, event), ["eventId"]),
        outputs: listed
      },
      %{
        name: "delete_event",
        description:
          "Deletes an event for good (Calendar's events.delete): a repeating event's id deletes every repeat, and one repeat's id just that one. Do it only when the person asked. Guests are told only with sendUpdates.",
        inputs: with_account.(%{eventId: event, calendarId: calendar, sendUpdates: send_updates}, ["eventId"]),
        outputs: %{
          type: "object",
          properties: %{id: string.("The event deleted"), summary: %{type: ["string", "null"]}, start: %{time.("When it started", "") | type: ["object", "null"]}},
          required: ["id", "summary", "start"]
        }
      }
    ]
  end

  @impl true
  def call(tool, inputs, call) when tool in ~w(accounts sign_in),
    do: google(call, tool, Map.put(inputs, "scopes", google().scopes))

  def call(tool, inputs, call) do
    run(tool, inputs, %{call: call, account: inputs["account"]})
  end

  defp run("calendars", _inputs, at) do
    calendars = api(at, "GET", "users/me/calendarList", query: [maxResults: 250])["items"] || []

    calendars =
      for calendar <- Enum.sort_by(calendars, &{&1["primary"] != true, &1["summary"] || ""}) do
        %{
          id: calendar["id"],
          summary: calendar["summaryOverride"] || calendar["summary"],
          primary: calendar["primary"] == true,
          accessRole: calendar["accessRole"],
          timeZone: calendar["timeZone"],
          hidden: calendar["selected"] == false
        }
      end

    %{calendars: calendars}
  end

  defp run("events", inputs, at) do
    zone = if inputs["timeMin"] || inputs["timeMax"], do: zone(at, inputs["calendarId"])
    from = instant(inputs["timeMin"], zone, DateTime.utc_now())
    to = instant(inputs["timeMax"], zone, DateTime.add(from, 7 * @day))

    found =
      api(at, "GET", "calendars/#{calendar(at, inputs["calendarId"])}/events",
        query: [
          timeMin: DateTime.to_iso8601(from),
          timeMax: DateTime.to_iso8601(to),
          q: inputs["q"],
          singleEvents: true,
          orderBy: "startTime",
          maxResults: min(inputs["maxResults"] || 50, 250),
          pageToken: inputs["pageToken"]
        ],
        missing: no_calendar(inputs["calendarId"])
      )

    %{
      calendar: found["summary"] || inputs["calendarId"] || "primary",
      timeZone: found["timeZone"],
      timeMin: DateTime.to_iso8601(from),
      timeMax: DateTime.to_iso8601(to),
      events: Enum.map(found["items"] || [], &listed/1),
      nextPageToken: found["nextPageToken"]
    }
  end

  defp run("event", %{"eventId" => id} = inputs, at) do
    whole(api(at, "GET", "calendars/#{calendar(at, inputs["calendarId"])}/events/#{path(at, id)}", missing: no_event(id, inputs["calendarId"])))
  end

  defp run("create_event", inputs, at) do
    event =
      api(at, "POST", "calendars/#{calendar(at, inputs["calendarId"])}/events",
        json: body(at, inputs, :new),
        query: [sendUpdates: inputs["sendUpdates"] || "none"],
        missing: no_calendar(inputs["calendarId"])
      )

    listed(event)
  end

  defp run("update_event", %{"eventId" => id} = inputs, at) do
    event =
      api(at, "PATCH", "calendars/#{calendar(at, inputs["calendarId"])}/events/#{path(at, id)}",
        json: body(at, inputs, :change),
        query: [sendUpdates: inputs["sendUpdates"] || "none"],
        missing: no_event(id, inputs["calendarId"])
      )

    listed(event)
  end

  defp run("delete_event", %{"eventId" => id} = inputs, at) do
    where = "calendars/#{calendar(at, inputs["calendarId"])}/events/#{path(at, id)}"
    event = api(at, "GET", where, missing: no_event(id, inputs["calendarId"]))
    if event["status"] == "cancelled", do: raise(no_event(id, inputs["calendarId"]))
    api(at, "DELETE", where, query: [sendUpdates: inputs["sendUpdates"] || "none"], missing: no_event(id, inputs["calendarId"]))
    %{id: event["id"], summary: event["summary"], start: event["start"]}
  end

  # The Event resource events.insert and events.patch take, from what was given: a dateTime with neither an offset
  # nor a timeZone is in the calendar's zone, and a new event ends an hour after it starts, or a day.
  defp body(at, inputs, new_or_change) do
    body = Map.take(inputs, ~w(summary location description attendees recurrence start end))

    body =
      if Enum.any?([body["start"], body["end"]], &zoneless?/1) do
        zone = zone(at, inputs["calendarId"])
        for key <- ~w(start end), zoneless?(body[key]), reduce: body, do: (body -> put_in(body, [key, "timeZone"], zone))
      else
        body
      end

    case {new_or_change, body} do
      {:new, %{"start" => %{"date" => date}} = body} when not is_map_key(body, "end") ->
        Map.put(body, "end", %{"date" => Date.to_iso8601(Date.add(Date.from_iso8601!(date), 1))})

      {:new, %{"start" => %{"dateTime" => start} = time} = body} when not is_map_key(body, "end") ->
        Map.put(body, "end", %{"dateTime" => DateTime.to_iso8601(DateTime.add(instant(start, time["timeZone"], nil), 3600))})

      {_, body} ->
        body
    end
  end

  defp zoneless?(%{"dateTime" => time} = given) when is_binary(time), do: given["timeZone"] == nil and not offset?(time)
  defp zoneless?(_time), do: false

  defp offset?(time), do: String.trim(time) =~ ~r/(Z|[+-]\d{2}:?\d{2})$/i

  # An instant from what was given: a time with its offset, or one without, or a date, in the calendar's zone.
  defp instant(nil, _zone, fallback), do: fallback

  defp instant(given, zone, _fallback) do
    given = String.trim(given)

    if offset?(given) do
      case DateTime.from_iso8601(given) do
        {:ok, at, _offset} -> at
        _ -> raise "#{given} isn't an RFC 3339 time, like 2026-11-02T09:30:00+11:00."
      end
    else
      wall =
        case NaiveDateTime.from_iso8601(if(given =~ ~r/^\d{4}-\d{2}-\d{2}$/, do: given <> "T00:00:00", else: given)) do
          {:ok, wall} -> wall
          _ -> raise "#{given} isn't a time or a date, like 2026-11-02T09:30:00 or 2026-11-02."
        end

      case DateTime.from_naive(wall, zone || "Etc/UTC") do
        {:ok, at} -> DateTime.shift_zone!(at, "Etc/UTC")
        # A time a clock skips, or passes twice, as it changes.
        {:gap, _before, after_} -> DateTime.shift_zone!(after_, "Etc/UTC")
        {:ambiguous, first, _second} -> DateTime.shift_zone!(first, "Etc/UTC")
        {:error, _why} -> raise "#{zone} isn't a time zone."
      end
    end
  end

  defp zone(at, calendar) do
    api(at, "GET", "calendars/#{calendar(at, calendar)}", missing: no_calendar(calendar))["timeZone"]
  end

  @listed ~w(id status summary start end location recurringEventId attendees htmlLink)
  @people ~w(email displayName responseStatus optional)

  # An event as the tools answer it, from Calendar's Event resource: what `tools/0` says of it.
  defp listed(event) do
    event = Map.merge(Map.new(@listed, &{&1, nil}), Map.take(event, @listed))
    %{event | "attendees" => for(who <- event["attendees"] || [], do: Map.take(who, @people))}
  end

  defp whole(event) do
    Map.merge(listed(event), %{
      "description" => event["description"],
      "recurrence" => event["recurrence"] || [],
      "organizer" => event["organizer"] && Map.take(event["organizer"], ["email", "displayName"]),
      "hangoutLink" => event["hangoutLink"]
    })
  end

  defp api(at, method, path, options), do: request(at, method, @api <> path, options)["body"]

  defp calendar(at, id), do: path(at, id || "primary")
  # A part of a URL's path, as Google takes it: an id or a range, with anything but letters and digits escaped.
  defp path(_at, part), do: URI.encode(to_string(part), &URI.char_unreserved?/1)

  # Google's API as the account `at` names, through google-account, which gives the answer's status, headers and
  # body, or raises with what went wrong.
  defp request(at, method, url, options) do
    given = for {option, value} <- options, value != nil, into: %{}, do: {to_string(option), value}
    inputs = Map.merge(given, %{"method" => method, "url" => url})
    google(at.call, "request", if(at.account, do: Map.put(inputs, "account", at.account), else: inputs))
  end

  # Google's sign-in, the accounts and Google's API are google-account's, whose tools act for this plugin.
  defp google(call, tool, inputs) do
    inputs = Map.new(Map.put(inputs, "title", google().title), &query/1)
    OneCore.Call.tool(call, "google-account", tool, JSON.decode!(JSON.encode!(inputs)))
  end

  defp query({"query", query}), do: {"query", Map.new(query)}
  defp query(input), do: input

  defp no_calendar(id), do: "No calendar #{id}: calendar ids come from google-calendar_calendars."

  defp no_event(id, calendar),
    do:
      "No event #{id} in #{if calendar, do: "calendar #{calendar}", else: "your primary calendar"}: event ids come from google-calendar_events, with the same calendarId."
end
