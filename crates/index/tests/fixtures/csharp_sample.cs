// SPDX-License-Identifier: Apache-2.0
using System;
using System.Collections.Generic;
using static System.Math;
using Json = Newtonsoft.Json.JsonConvert;

namespace Shop.Billing
{
    /// <summary>
    /// A customer invoice.
    /// </summary>
    /// <remarks>Immutable once issued.</remarks>
    [Serializable]
    public class Invoice<T> : BaseInvoice, IPriced, IDisposable where T : class
    {
        public const int MaxLines = 500;
        private static readonly string Prefix = "inv-";
        private int lineCount;
        public string Customer { get; set; }

        /// <summary>Creates an invoice.</summary>
        public Invoice(string customer)
        {
            Customer = customer;
        }

        /// <summary>Adds a line &amp; recomputes.</summary>
        [Obsolete("use AddLine")]
        public virtual void Add(Line line)
        {
            Validate(line);
            var copy = new Line(line.Sku, line.Qty);
            Console.WriteLine(Format(copy));
            Logger.Info("added");
        }

        internal static async Task<int> CountAsync(IEnumerable<Line> lines)
        {
            return await Task.FromResult(lines.Count());
        }

        private void Validate(Line line) { }

        protected decimal Total() => Max(1, 2);

        public event EventHandler Changed;

        public enum Status { Open, Closed }

        public void Dispose() { }
    }

    public interface IPriced : IComparable
    {
        decimal Price();
    }

    public struct Line
    {
        public string Sku;
        public int Qty;
    }

    public record Money(decimal Amount, string Currency);

    public delegate void Notify(string message);

    internal static class Helpers
    {
        public static string Format(Line line) => line.Sku;
    }
}
