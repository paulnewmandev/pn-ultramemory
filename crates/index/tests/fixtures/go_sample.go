// SPDX-License-Identifier: Apache-2.0

// Package billing computes invoices. This fixture is used by the extraction tests.
package billing

import (
	"errors"
	"fmt"
	str "strings"
	_ "embed"
)

import "os"

// MaxLines is the largest number of lines on one invoice.
const MaxLines = 500

const (
	statusOpen = iota
	StatusPaid
)

// DefaultCurrency is used when none is given.
var DefaultCurrency = "EUR"

var (
	cache map[string]*Invoice
	Debug bool
)

// Priced is implemented by everything that has a price.
type Priced interface {
	Price() int64
	fmt.Stringer
}

// Line is one line of an invoice.
type Line struct {
	SKU   string
	Qty   int
	price int64
}

// Invoice groups lines for one customer.
type Invoice struct {
	Line
	*Customer
	Lines []Line
	total int64
}

// Currency is an ISO currency code.
type Currency string

type Alias = Invoice

type handler func(*Invoice) error

// New creates an empty invoice.
func New(customer *Customer) *Invoice {
	fmt.Println("new invoice")
	return &Invoice{Customer: customer}
}

// Add appends a line and updates the total.
func (inv *Invoice) Add(l Line) error {
	if len(inv.Lines) >= MaxLines {
		return errors.New("too many lines")
	}
	inv.Lines = append(inv.Lines, l)
	inv.recompute()
	return nil
}

func (inv Invoice) recompute() {
	var sum int64
	for _, l := range inv.Lines {
		sum += l.price * int64(l.Qty)
	}
	inv.total = sum
	_ = str.ToUpper(string(inv.SKU))
}

// Format renders an invoice.
func Format[T Priced](items []T) string {
	return os.Getenv("HOME") + fmt.Sprint(len(items))
}

func helper() {}
