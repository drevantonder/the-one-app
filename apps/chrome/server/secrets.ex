defmodule Chrome.Secrets do
  @moduledoc """
  Hides what's shaped like a secret in the text of a page, as `snapshot` and the other tools that answer with it
  give it: an agent reads it, and any site may show one (a Copy button's name holds a client secret, a page
  prints a token it has just made).

  The patterns are Gitleaks' default rules, `server/gitleaks.json`, which is its `config/gitleaks.toml` (MIT, pinned
  to the tag its `source` names) as JSON, since an app has no TOML parser. To update it, fetch that file at a newer tag
  and run it through Python's `tomllib` into `server/gitleaks.json` the way `rules` here reads it: each rule's
  `id`, `regex`, `keywords`, `entropy`, `secretGroup` and the regexes of its allowlists, and the global allowlist.

  A rule is applied as Gitleaks does: only where one of its keywords is in the text (which is what keeps this cheap),
  and what its regex matches, in its `secretGroup` (else its first group, else the whole match), is a secret unless its
  entropy is under the rule's or an allowlist's regex matches it. Each secret becomes `••••••`.
  """

  @mask "••••••"

  # What Gitleaks has no rule for.
  @more [
    # Google's OAuth client secrets (GitHub's secret scanning documents the format).
    %{"id" => "google-oauth-client-secret", "regex" => "GOCSPX-[A-Za-z0-9_-]{28}", "keywords" => ["gocspx-"]}
  ]

  # Gitleaks' guess at a secret from the words around it ("key", "token", "auth" and a value after a colon), made for
  # source code. In a page's text those are the labels and sentences of ordinary pages.
  @generic "generic-api-key"

  # How far from a keyword Gitleaks' regexes look: up to 100 bytes before it (they put at most 50 of a name before
  # one) and, after it, as long as a private key is.
  @lead 100
  @reach 8192

  @doc "`text` with each secret in it as `#{@mask}`."
  def mask(text) do
    %{"rules" => rules, "allow" => allow} = file()
    lower = String.downcase(text, :ascii)

    secrets =
      for rule <- rules ++ @more,
          rule["id"] != @generic,
          window <- windows(rule["keywords"], lower),
          secret <- secrets(rule, allow, text, window),
          do: secret

    secrets |> Enum.sort() |> merge() |> hide(text)
  end

  # Where a rule looks in `text`: around each of its keywords, as {byte offset, length}.
  defp windows(keywords, lower) do
    windows =
      for keyword <- keywords,
          {at, size} <- :binary.matches(lower, keyword),
          from = max(at - @lead, 0),
          do: {from, min(at + size + @reach, byte_size(lower)) - from}

    windows |> Enum.sort() |> merge()
  end

  defp hide(ranges, text) do
    {parts, at} =
      Enum.reduce(ranges, {[], 0}, fn {from, length}, {parts, at} ->
        {[parts, binary_part(text, at, from - at), @mask], from + length}
      end)

    IO.iodata_to_binary([parts, binary_part(text, at, byte_size(text) - at)])
  end

  defp file, do: JSON.decode!(OneCore.App.file(__MODULE__, "server/gitleaks.json"))

  # Where in a window of `text` (byte offset, length) a rule finds secrets, as offsets and lengths in `text`.
  defp secrets(rule, allow, text, {at, size}) do
    regex = Regex.compile!(rule["regex"])
    group = rule["secretGroup"]
    window = binary_part(text, at, size)

    for [{start, size} = whole | groups] <- Regex.scan(regex, window, return: :index),
        {from, length} = secret_at(whole, groups, group),
        length > 0,
        secret = binary_part(window, from, length),
        entropy(secret) >= (rule["entropy"] || 0),
        not allowed?(secret, binary_part(window, start, size), rule["allow"] || [], allow),
        do: {at + from, length}
  end

  defp secret_at(whole, _groups, 0), do: whole
  defp secret_at(_whole, groups, group) when is_integer(group), do: Enum.at(groups, group - 1)
  defp secret_at(_whole, [first | _], nil), do: first
  defp secret_at(whole, [], nil), do: whole

  defp allowed?(secret, match, allowed, global) do
    Enum.any?(global["stopwords"], &String.contains?(String.downcase(secret), &1)) or
      Enum.any?(global["regexes"], &(secret =~ Regex.compile!(&1))) or
      Enum.any?(allowed, fn %{"regexes" => regexes, "target" => target} ->
        Enum.any?(regexes, &((if target == "match", do: match, else: secret) =~ Regex.compile!(&1)))
      end)
  end

  # Shannon entropy, bits per byte.
  defp entropy(secret) do
    size = byte_size(secret)

    secret
    |> :binary.bin_to_list()
    |> Enum.frequencies()
    |> Enum.reduce(0.0, fn {_byte, count}, bits -> bits - count / size * :math.log2(count / size) end)
  end

  # Overlapping or touching ranges as one.
  defp merge([{from, length}, {next, more} | rest]) when next <= from + length,
    do: merge([{from, max(length, next + more - from)} | rest])

  defp merge([range | rest]), do: [range | merge(rest)]
  defp merge([]), do: []
end
