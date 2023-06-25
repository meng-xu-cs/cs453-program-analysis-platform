Welcome to The Grading Platform
===============================

TLDR;
```
POST ugster71a.student.cs.uwaterloo.ca:8000/submit          # submit package
GET  ugster71a.student.cs.uwaterloo.ca:8000/status/<hash>   # retrieve status
```

# To submit a package for analysis

```
POST ugster71a.student.cs.uwaterloo.ca:8000/submit
```

Include a ZIP-ed archived as binary data in the body of the POST request.

If you work on a UNIX system, you can use the following command to submit a
package from the terminal:

```bash
zip -r - <path-to-package>/* | curl --data-binary @- ugster71a.student.cs.uwaterloo.ca:8000/submit
```

Upon submission, you will receive a message indicating that the package is in
one of the following status:
- malformed, with an explanation on why it is invalid
- scheduled for analysis, or
- has been submitted before and its status can be retrieved

In the latter two cases, you will be provided a hash of the package (a URL to
its status page actually).

# To retrieve the status of a submitted package

```
GET ugster71a.student.cs.uwaterloo.ca:8000/status/<hash>
```

You will see one of the following responses:
- A display of the analysis result
- Queued, with a position in the queue
- A display of an error encountered in the analysis.
  If you think the error is not caused by your mistake, make a post on Piazza.
