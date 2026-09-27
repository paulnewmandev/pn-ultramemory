# SPDX-License-Identifier: Apache-2.0

defmodule MyApp.Worker do
  @moduledoc """
  Does background work.

  Fixture for the extraction tests.
  """

  use GenServer

  @doc "Starts the worker."
  def start_link(opts) do
    GenServer.start_link(__MODULE__, opts, name: __MODULE__)
  end

  @doc """
  Runs a job.

  Returns the result.
  """
  @spec run(map()) :: :ok
  def run(job) do
    helper(job)
    Enum.map(job.items, &format/1)
  end

  def short(a), do: a + 1

  defp helper(job) do
    IO.inspect(job)
  end

  defmacro debug(expr) do
    quote do
      IO.inspect(unquote(expr))
    end
  end

  defmodule Nested do
    def inner, do: :ok
  end
end

defprotocol Sizeable do
  def size(value)
end
