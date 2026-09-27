<?php
// SPDX-License-Identifier: Apache-2.0
declare(strict_types=1);

namespace Shop\Billing;

use Shop\Contracts\Priced;
use Shop\Support\{Money, Formatter as Fmt};
use function Shop\Support\round_cents;
require_once 'vendor/autoload.php';
include "helpers.php";

const DEFAULT_CURRENCY = 'EUR';

/**
 * A customer invoice.
 *
 * Holds lines until it is issued.
 */
#[Entity(table: 'invoices')]
abstract class Invoice extends Model implements Priced, \Countable
{
    use Timestamps, SoftDeletes;

    public const MAX_LINES = 500;
    private const SECRET = 'x';
    private int $count = 0;
    public static ?Invoice $last = null;

    /** Creates an invoice. */
    public function __construct(private string $customer, protected ?Money $total = null)
    {
        parent::__construct();
    }

    /**
     * Adds a line.
     */
    public function addLine(Line $line): static
    {
        $this->validate($line);
        self::log("added");
        $copy = new Line($line->sku, 1);
        return round_cents($this->total) ? $this : $copy;
    }

    public static function fromArray(array $data): self
    {
        return static::query()->where('id', $data['id'])->first();
    }

    protected function validate(Line $line): void
    {
        assert($line->qty > 0);
    }

    private function secret(): int { return 1; }

    abstract public function total(): Money;
}

interface Priced2 extends Base
{
    public function price(): int;
}

trait Timestamps
{
    public function touch(): void {}
}

enum Status: string
{
    case Open = 'open';

    public function label(): string
    {
        return ucfirst($this->value);
    }
}

function format_invoice(Invoice $invoice, array $options = []): string
{
    return Fmt::render($invoice, count($options));
}
