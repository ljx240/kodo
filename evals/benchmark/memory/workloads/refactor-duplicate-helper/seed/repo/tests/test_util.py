from util import format_currency, money_str


def test_format_currency_default():
    assert format_currency(1234.5) == "1,234.50 USD"


def test_money_str_default():
    assert money_str(1234.5) == "1,234.50 EUR"
