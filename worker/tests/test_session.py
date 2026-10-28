import base64

from nacl.public import PrivateKey

from jumbo_worker.session import Session, SessionTicket

SID = "0123456789abcdef0123456789abcdef"


def test_idle_and_deadline_clocks():
    now = [1000.0]
    ticket = SessionTicket.from_dict({"proto": 1, "session_id": SID, "app_pk": base64.b64encode(bytes(PrivateKey.generate().public_key)).decode(), "deadline_s": 100, "idle_s": 30})
    session = Session(ticket, clock=lambda: now[0])
    now[0] += 29
    assert session.should_stop() is None
    now[0] += 1
    assert "idle" in session.should_stop()
    session.last_seen = now[0]
    now[0] = 1100
    assert session.should_stop() == "deadline reached"


def test_a_session_the_app_never_reaches_stops_early():
    now = [1000.0]
    ticket = SessionTicket.from_dict({"proto": 1, "session_id": SID, "app_pk": base64.b64encode(bytes(PrivateKey.generate().public_key)).decode(), "idle_s": 1800, "first_contact_s": 600})
    session = Session(ticket, clock=lambda: now[0])
    now[0] += 599
    assert session.should_stop() is None
    now[0] += 1
    assert session.should_stop() == "the app never connected"
    session.contacted = True
    assert session.should_stop() is None
