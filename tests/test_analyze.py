from unittest.mock import MagicMock
import pytest
from git_of_theseus.analyze import get_mailmap_author_name_email


@pytest.fixture(autouse=True)
def clear_cache():
    get_mailmap_author_name_email.cache_clear()
    yield
    get_mailmap_author_name_email.cache_clear()


def _make_repo(check_mailmap_output):
    repo = MagicMock()
    repo.git.check_mailmap.return_value = check_mailmap_output
    return repo


def test_normal_name_and_email():
    repo = _make_repo("Alice Smith <alice@example.com>")
    name, email = get_mailmap_author_name_email(repo, "Alice", "alice@example.com")
    assert name == "Alice Smith"
    assert email == "alice@example.com"


def test_mailmap_remaps_name_and_email():
    repo = _make_repo("Bob Jones <bob@canonical.com>")
    name, email = get_mailmap_author_name_email(repo, "bob", "bob@old.com")
    assert name == "Bob Jones"
    assert email == "bob@canonical.com"


def test_no_email_in_output():
    # git check-mailmap returns just a name with no <email> part
    repo = _make_repo("Charlie")
    name, email = get_mailmap_author_name_email(repo, "Charlie", "charlie@example.com")
    assert name == "Charlie"
    # Falls back to the original author_email
    assert email == "charlie@example.com"
