# As the server whose home `:home` is, with no credentials but its key: writes and reads a blob through what the
# signer signs, shows the bucket itself refusing a second write and a write whose condition was dropped, and times
# a write. Run with `mix run --no-start`.
Application.ensure_all_started(:req)
{_, address} = Application.fetch_env!(:one_core, :bucket)
alias OneCore.{Address, Bucket}
key = "users/anyone/proof/#{System.unique_integer([:positive])}-#{System.os_time()}"

IO.puts("write only if absent: #{inspect(Bucket.write(key, "first"))}")
IO.puts("the same write again: #{inspect(Bucket.write(key, "second"))}")
{:ok, data, tag} = Bucket.read(key)
IO.puts("read: #{inspect(data)}, tag #{tag}")
IO.puts("replace only if unchanged: #{inspect(Bucket.replace(key, "third", tag))}")
IO.puts("the same replace again: #{inspect(Bucket.replace(key, "fourth", tag))}")
IO.puts("list: #{inspect(Bucket.list(key))}")

# What the signer hands back for a write, used as its holder might try to.
ask = JSON.encode!(%{write: key})
%{body: %{"url" => url, "headers" => headers}} =
  Req.post!(address <> "/signer/sign", body: ask, headers: Address.signed("/signer/sign", ask))

put = fn url, headers -> Req.put!(url, headers: headers, body: "overwritten", decode_body: false, retry: false) end
said = fn %{status: status, body: body} -> "#{status} #{Regex.run(~r/<Code>(.*?)<\/Code>/, body, capture: :all_but_first)}" end
IO.puts("\nthe signed write is #{String.replace(url, ~r/Signature=.*/, "Signature=…")} with #{inspect(headers)}")
IO.puts("sent as signed, the bucket answers: #{said.(put.(url, headers))}")
IO.puts("with its condition left off: #{said.(put.(url, %{}))}")
unlisted = String.replace(url, "host%3Bif-none-match", "host")
IO.puts("with its condition left off and taken out of what's listed as signed: #{said.(put.(unlisted, %{}))}")
IO.puts("with another condition: #{said.(put.(url, %{"if-none-match" => "\"never\""}))}")
IO.puts("to another blob: #{said.(put.(String.replace(url, "/proof/", "/other/"), headers))}")
{:ok, data, _} = Bucket.read(key)
IO.puts("the blob is still #{inspect(data)}")

blob = :crypto.strong_rand_bytes(4096)
time = fn work -> :timer.tc(work) |> elem(0) end
writes = for n <- 1..20, do: time.(fn -> :ok = Bucket.write("#{key}/#{n}", blob) end)
signs = for _ <- 1..20, do: time.(fn -> Req.post!(address <> "/signer/sign", body: ask, headers: Address.signed("/signer/sign", ask)) end)
middle = fn times -> Float.round(Enum.at(Enum.sort(times), 10) / 1000, 1) end
IO.puts("\na 4 KB write through signed access: middle #{middle.(writes)} ms of 20, of which asking the signer is #{middle.(signs)} ms")
