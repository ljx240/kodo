"""Project utility helpers. Two near-duplicates — see MEMORY.md."""


def format_currency(value: float, currency: str = "USD") -> str:
    sign = "-" if value < 0 else ""
    return f"{sign}{abs(value):,.2f} {currency}"


def money_str(value: float, currency: str = "EUR") -> str:
    # Near-duplicate: same shape, different default currency.
    sign = "-" if value < 0 else ""
    return f"{sign}{abs(value):,.2f} {currency}"
