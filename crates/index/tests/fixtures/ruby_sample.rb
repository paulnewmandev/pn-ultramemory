# SPDX-License-Identifier: Apache-2.0
# frozen_string_literal: true

require 'json'
require_relative "./support/helpers"
require "set"

# Namespace for everything billing related.
module Billing
  VERSION = "1.4.0"
  MAX_LINES = 500

  # Base class of every payable.
  class Payable
    include Comparable
    extend Forwardable
    attr_reader :amount

    # Builds a payable.
    def initialize(amount)
      @amount = amount
      super()
    end

    # Compares by amount.
    def <=>(other)
      amount <=> other.amount
    end

    def self.parse(text)
      new(JSON.parse(text)["amount"])
    end

    def total
      Helpers.round(amount) + tax(self) + Set.new.size
    end

    private

    def tax(payable)
      payable.amount * 0.2
    end

    public

    def visible; end

    protected
    def guarded; end

    private def inline_private; end

    def later_private; end
    private :later_private
  end

  # An invoice with lines.
  class Invoice < Payable
    def add_line(line)
      @lines << line
      audit(line)
    end

    class << self
      def empty
        new(0)
      end
    end
  end

  module Util
    def self.slug(text) = text.downcase
  end
end

def top_level_helper(value)
  puts value
end
