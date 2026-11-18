defmodule Vault.PublicSuffix do
  @moduledoc """
  The registrable domain of a host (its eTLD+1), by the Public Suffix List as browsers read it for cookies:
  `oauth2.googleapis.com` is under `googleapis.com`, and `a.example.co.uk` under `example.co.uk`. The list is
  `server/public_suffix_list.dat`, publicsuffix.org's, as a Hex package would carry it: an app can't depend on one,
  and a newer file replaces it. Only its ICANN section counts, as tldextract's default does: the private section
  isolates cookies between a host's tenants, and lists a service's own API domain (`googleapis.com`), whose
  sites are one service.
  """

  @doc "The registrable domain of `host`, or nil for an IP address or a host that's a public suffix itself."
  def registrable(host) do
    labels = host |> String.downcase() |> String.trim_trailing(".") |> String.split(".")

    if match?({:ok, _}, :inet.parse_address(String.to_charlist(host))) or "" in labels do
      nil
    else
      rules = rules()
      count = suffix_length(labels, rules)
      if length(labels) > count, do: labels |> Enum.take(-(count + 1)) |> Enum.join(".")
    end
  end

  # The algorithm publicsuffix.org gives: the matching exception rule wins, and its suffix is one label shorter;
  # otherwise the longest matching rule, a wildcard standing for any one label; with none, the last label.
  defp suffix_length(labels, {exact, wild, except}) do
    suffixes = for n <- 1..length(labels), do: {n, labels |> Enum.take(-n) |> Enum.join(".")}

    case Enum.find(suffixes, fn {_n, suffix} -> suffix in except end) do
      {n, _suffix} ->
        n - 1

      nil ->
        matched =
          for {n, suffix} <- suffixes,
              suffix in exact or (n > 1 and suffix |> String.split(".", parts: 2) |> List.last() |> then(&(&1 in wild))),
              do: n

        Enum.max(matched, fn -> 1 end)
    end
  end

  defp rules do
    case :persistent_term.get(__MODULE__, nil) do
      nil ->
        lines =
          OneCore.App.file(__MODULE__, "server/public_suffix_list.dat")
          |> String.split("// ===END ICANN DOMAINS===")
          |> hd()
          |> String.split("\n", trim: true)
          |> Enum.map(&(&1 |> String.split(~r/\s/, parts: 2) |> hd()))
          |> Enum.reject(&(&1 == "" or String.starts_with?(&1, "//")))

        rules = {
          MapSet.new(for(line <- lines, not String.starts_with?(line, ["*.", "!"]), do: line)),
          MapSet.new(for("*." <> rest <- lines, do: rest)),
          MapSet.new(for("!" <> rest <- lines, do: rest))
        }

        :persistent_term.put(__MODULE__, rules)
        rules

      rules ->
        rules
    end
  end
end
