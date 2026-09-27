-- SPDX-License-Identifier: Apache-2.0
-- Utility module used by the extraction tests.
local M = {}

--- Adds two numbers.
-- The second line of the documentation.
function M.add(a, b)
  local r = helper(a) + b
  return math.max(r, 0)
end

local function helper(x)
  return x * 2
end

function M:method(arg)
  print("hi")
  self:other(arg)
end

--[[ Block
documentation ]]
function documented_block() end

local Class = {}
Class.__index = Class

function Class.new(name)
  local self = setmetatable({}, Class)
  self.name = name
  return self
end

function Class:greet()
  return string.format("hello %s", self.name)
end

return M
